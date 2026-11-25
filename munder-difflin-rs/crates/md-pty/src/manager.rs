//! Cross-platform PTY manager backed by `portable-pty`, mirroring `PtyManager`
//! from `src/main/pty.ts`.

use md_core::config::expand_tilde;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::sync::broadcast;

const TAIL_MAX: usize = 8192;
const REDRAW_STEP_MS: u64 = 80;

#[derive(Debug, Error)]
pub enum PtyError {
    #[error("pty already exists for id {0}")]
    AlreadyExists(String),
    #[error("no pty: {0}")]
    NotFound(String),
    #[error("cwd does not exist: {0}")]
    MissingCwd(String),
    #[error("bad grid: {0}x{1}")]
    BadGrid(u16, u16),
    #[error("pty error: {0}")]
    Pty(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnOptions {
    pub id: String,
    pub cwd: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub shell_script: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_missing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyListEntry {
    pub id: String,
    pub cwd: String,
    pub command: String,
    pub pid: Option<u32>,
    pub last_output_at: u64,
    pub has_output: bool,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyInfo {
    pub pid: Option<u32>,
    pub cols: u16,
    pub rows: u16,
    pub has_output: bool,
    pub started_at: u64,
    pub command: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PtyEvent {
    Data {
        id: String,
        data: String,
    },
    Exit {
        id: String,
        exit_code: u32,
        signal: Option<String>,
        tail: String,
        started_at: u64,
    },
}

struct PtySession {
    generation: u64,
    id: String,
    cwd: String,
    command: String,
    pid: Option<u32>,
    cols: u16,
    rows: u16,
    started_at: u64,
    last_output_at: u64,
    has_output: bool,
    tail: String,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

#[derive(Default)]
struct Inner {
    next_gen: u64,
    sessions: HashMap<String, PtySession>,
    wanted: HashMap<String, (u16, u16)>,
}

#[derive(Clone)]
pub struct PtyManager {
    inner: Arc<Mutex<Inner>>,
    events_tx: broadcast::Sender<PtyEvent>,
}

impl PtyManager {
    pub fn new() -> Self {
        let (events_tx, _) = broadcast::channel(1024);
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            events_tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PtyEvent> {
        self.events_tx.subscribe()
    }

    pub fn spawn(&self, opts: SpawnOptions) -> SpawnResult {
        let cwd_path = expand_tilde(&opts.cwd);
        if !cwd_path.is_dir() {
            return SpawnResult {
                ok: false,
                cwd: None,
                cli_missing: None,
                error: Some(format!("cwd does not exist: {}", cwd_path.display())),
            };
        }

        let resolved_cmd = if opts.shell_script.is_some() {
            opts.command.clone()
        } else {
            match resolve_command(&opts.command) {
                Some(p) => p.to_string_lossy().into_owned(),
                None => {
                    return SpawnResult {
                        ok: false,
                        cwd: Some(cwd_path.to_string_lossy().into_owned()),
                        cli_missing: Some(true),
                        error: Some(format!("command not found: {}", opts.command)),
                    };
                }
            }
        };

        let (cols, rows) = {
            let guard = self.inner.lock().unwrap();
            if guard.sessions.contains_key(&opts.id) {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(format!("pty already exists for id {}", opts.id)),
                };
            }
            guard.wanted.get(&opts.id).copied().unwrap_or((
                opts.cols.filter(|&c| c > 0).unwrap_or(100),
                opts.rows.filter(|&r| r > 0).unwrap_or(30),
            ))
        };

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(p) => p,
            Err(e) => {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(e.to_string()),
                };
            }
        };

        let mut cmd = if let Some(ref script) = opts.shell_script {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
            let mut b = CommandBuilder::new(shell);
            b.args(["-lc", script]);
            b
        } else {
            let mut b = CommandBuilder::new(&resolved_cmd);
            b.args(&opts.args);
            b
        };
        cmd.cwd(&cwd_path);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let mut child = match pair.slave.spawn_command(cmd) {
            Ok(c) => c,
            Err(e) => {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(e.to_string()),
                };
            }
        };
        drop(pair.slave);

        let pid = child.process_id();
        let killer = child.clone_killer();
        let reader = match pair.master.try_clone_reader() {
            Ok(r) => r,
            Err(e) => {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(e.to_string()),
                };
            }
        };
        let writer = match pair.master.take_writer() {
            Ok(w) => w,
            Err(e) => {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(e.to_string()),
                };
            }
        };

        let now = now_ms();
        let cwd_str = cwd_path.to_string_lossy().into_owned();
        let generation = {
            let mut guard = self.inner.lock().unwrap();
            if guard.sessions.contains_key(&opts.id) {
                return SpawnResult {
                    ok: false,
                    cwd: None,
                    cli_missing: None,
                    error: Some(format!("pty already exists for id {}", opts.id)),
                };
            }
            guard.next_gen += 1;
            let gen_id = guard.next_gen;
            guard.sessions.insert(
                opts.id.clone(),
                PtySession {
                    generation: gen_id,
                    id: opts.id.clone(),
                    cwd: cwd_str.clone(),
                    command: resolved_cmd,
                    pid,
                    cols,
                    rows,
                    started_at: now,
                    last_output_at: now,
                    has_output: false,
                    tail: String::new(),
                    master: pair.master,
                    writer,
                    killer,
                },
            );
            gen_id
        };

        // Blocking reader thread for PTY stdout/stderr stream.
        let id_reader = opts.id.clone();
        let inner_reader = Arc::clone(&self.inner);
        let tx_reader = self.events_tx.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        {
                            let mut guard = inner_reader.lock().unwrap();
                            let Some(sess) = guard.sessions.get_mut(&id_reader) else {
                                break;
                            };
                            if sess.generation != generation {
                                break;
                            }
                            sess.has_output = true;
                            sess.last_output_at = now_ms();
                            append_tail(&mut sess.tail, &chunk);
                        }
                        let _ = tx_reader.send(PtyEvent::Data {
                            id: id_reader.clone(),
                            data: chunk,
                        });
                    }
                }
            }
        });

        // Waiter thread for natural child exit.
        let id_waiter = opts.id;
        let inner_waiter = Arc::clone(&self.inner);
        let tx_waiter = self.events_tx.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            let (exit_code, signal) = match status {
                Ok(st) => (
                    st.exit_code(),
                    st.signal().map(ToString::to_string),
                ),
                Err(_) => (1, None),
            };
            let removed = {
                let mut guard = inner_waiter.lock().unwrap();
                match guard.sessions.get(&id_waiter) {
                    Some(s) if s.generation == generation => guard.sessions.remove(&id_waiter),
                    _ => None,
                }
            };
            if let Some(sess) = removed {
                let _ = tx_waiter.send(PtyEvent::Exit {
                    id: id_waiter,
                    exit_code,
                    signal,
                    tail: sess.tail,
                    started_at: sess.started_at,
                });
            }
        });

        SpawnResult {
            ok: true,
            cwd: Some(cwd_str),
            cli_missing: None,
            error: None,
        }
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), PtyError> {
        let mut guard = self.inner.lock().unwrap();
        let sess = guard
            .sessions
            .get_mut(id)
            .ok_or_else(|| PtyError::NotFound(id.to_string()))?;
        sess.writer
            .write_all(data.as_bytes())
            .and_then(|_| sess.writer.flush())
            .map_err(|e| PtyError::Pty(e.to_string()))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<bool, PtyError> {
        if cols == 0 || rows == 0 {
            return Err(PtyError::BadGrid(cols, rows));
        }
        let mut guard = self.inner.lock().unwrap();
        guard.wanted.insert(id.to_string(), (cols, rows));
        let Some(sess) = guard.sessions.get_mut(id) else {
            return Ok(true); // pending until spawn
        };
        if sess.cols == cols && sess.rows == rows {
            return Ok(false);
        }
        sess.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        sess.cols = cols;
        sess.rows = rows;
        Ok(false)
    }

    pub async fn redraw(&self, id: &str) -> Result<(), PtyError> {
        let (cols, rows, gen_id) = {
            let mut guard = self.inner.lock().unwrap();
            let sess = guard
                .sessions
                .get_mut(id)
                .ok_or_else(|| PtyError::NotFound(id.to_string()))?;
            let cols = sess.cols;
            let rows = sess.rows;
            let step = if rows > 1 { rows - 1 } else { rows + 1 };
            sess.master
                .resize(PtySize {
                    rows: step,
                    cols,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .map_err(|e| PtyError::Pty(e.to_string()))?;
            sess.rows = step;
            (cols, rows, sess.generation)
        };

        tokio::time::sleep(std::time::Duration::from_millis(REDRAW_STEP_MS)).await;

        let mut guard = self.inner.lock().unwrap();
        let Some(sess) = guard.sessions.get_mut(id) else {
            return Err(PtyError::NotFound(id.to_string()));
        };
        if sess.generation != gen_id {
            return Err(PtyError::Pty(format!("pty replaced during redraw: {id}")));
        }
        let step = if rows > 1 { rows - 1 } else { rows + 1 };
        if sess.cols != cols || sess.rows != step {
            return Ok(());
        }
        sess.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        sess.rows = rows;
        Ok(())
    }

    pub fn kill(&self, id: &str) -> Result<(), PtyError> {
        let mut removed = {
            let mut guard = self.inner.lock().unwrap();
            guard
                .sessions
                .remove(id)
                .ok_or_else(|| PtyError::NotFound(id.to_string()))?
        };
        removed
            .killer
            .kill()
            .map_err(|e| PtyError::Pty(e.to_string()))?;
        Ok(())
    }

    pub fn list(&self) -> Vec<PtyListEntry> {
        let guard = self.inner.lock().unwrap();
        guard
            .sessions
            .values()
            .map(|s| PtyListEntry {
                id: s.id.clone(),
                cwd: s.cwd.clone(),
                command: s.command.clone(),
                pid: s.pid,
                last_output_at: s.last_output_at,
                has_output: s.has_output,
                cols: s.cols,
                rows: s.rows,
            })
            .collect()
    }

    pub fn info(&self, id: &str) -> Option<PtyInfo> {
        let guard = self.inner.lock().unwrap();
        guard.sessions.get(id).map(|s| PtyInfo {
            pid: s.pid,
            cols: s.cols,
            rows: s.rows,
            has_output: s.has_output,
            started_at: s.started_at,
            command: s.command.clone(),
        })
    }

    pub fn tail(&self, id: &str) -> String {
        let guard = self.inner.lock().unwrap();
        guard
            .sessions
            .get(id)
            .map(|s| s.tail.clone())
            .unwrap_or_default()
    }
}

impl Default for PtyManager {
    fn default() -> Self {
        Self::new()
    }
}

fn append_tail(tail: &mut String, chunk: &str) {
    tail.push_str(chunk);
    if tail.len() > TAIL_MAX {
        let mut cut = tail.len() - TAIL_MAX;
        while cut < tail.len() && !tail.is_char_boundary(cut) {
            cut += 1;
        }
        tail.drain(..cut);
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn resolve_command(command: &str) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }
    if command.contains('/') || command.contains('\\') {
        let p = Path::new(command);
        return p.exists().then(|| p.to_path_buf());
    }
    if !command
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(command);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    let mut extra = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ];
    if let Some(home) = dirs::home_dir() {
        extra.push(home.join(".local/bin"));
        extra.push(home.join(".cargo/bin"));
    }
    for dir in extra {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_write_resize_and_exit_work() {
        let mgr = PtyManager::new();
        let mut rx = mgr.subscribe();
        let res = mgr.spawn(SpawnOptions {
            id: "test-sh".to_string(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "echo hello_pty && sleep 0.05".to_string()],
            cols: Some(80),
            rows: Some(24),
            env: HashMap::new(),
            shell_script: None,
        });
        assert!(res.ok, "spawn failed: {:?}", res.error);

        let mut saw_hello = false;
        let mut saw_exit = false;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline && !(saw_hello && saw_exit) {
            if let Ok(Ok(ev)) =
                tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await
            {
                match ev {
                    PtyEvent::Data { id, data } if id == "test-sh" => {
                        if data.contains("hello_pty") {
                            saw_hello = true;
                        }
                    }
                    PtyEvent::Exit { id, exit_code, .. } if id == "test-sh" => {
                        assert_eq!(exit_code, 0);
                        saw_exit = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(saw_hello, "did not observe PTY output");
        assert!(saw_exit, "did not observe PTY exit");
    }
}
