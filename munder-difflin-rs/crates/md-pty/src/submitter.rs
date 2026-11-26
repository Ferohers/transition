//! Serial PTY submitter and bidirectional terminal probe protocol,
//! mirroring `src/main/ptySubmit.ts` (`pty:submit`, `pty:withdraw`, `terminal:probe`).

use crate::manager::{PtyError, PtyManager};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;
use uuid::Uuid;

pub const PROBE_TIMEOUT_MS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtySubmitRequest {
    pub pty_id: String,
    pub agent_id: Option<String>,
    pub provider: String,
    pub text: String,
    pub source: String,
    pub settle_ms: Option<u64>,
    #[serde(default)]
    pub press_only: bool,
    pub ref_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub ok: bool,
    pub outcome: String,
    pub proof: String,
    pub presses: u32,
    pub ms: u64,
    pub reason: Option<String>,
    pub chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProbeRequest {
    pub id: String,
    pub pty_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProbeReply {
    pub id: String,
    pub lines: Option<Vec<String>>,
    #[serde(default)]
    pub safe: bool,
    #[serde(default = "default_true")]
    pub pooled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Default)]
pub struct TerminalProbes {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<TerminalProbeReply>>>>,
}

impl TerminalProbes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, pty_id: &str, cols: u16, rows: u16) -> (TerminalProbeRequest, oneshot::Receiver<TerminalProbeReply>) {
        let id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let req = TerminalProbeRequest {
            id,
            pty_id: pty_id.to_string(),
            cols,
            rows,
        };
        (req, rx)
    }

    pub fn resolve(&self, reply: TerminalProbeReply) -> bool {
        if let Some(tx) = self.pending.lock().unwrap().remove(&reply.id) {
            let _ = tx.send(reply);
            true
        } else {
            false
        }
    }

    pub async fn wait_with_timeout(
        &self,
        id: &str,
        rx: oneshot::Receiver<TerminalProbeReply>,
    ) -> Option<TerminalProbeReply> {
        match tokio::time::timeout(std::time::Duration::from_millis(PROBE_TIMEOUT_MS), rx).await {
            Ok(Ok(reply)) => Some(reply),
            _ => {
                self.pending.lock().unwrap().remove(id);
                None
            }
        }
    }
}

#[derive(Clone)]
pub struct PtySubmitter {
    pty: PtyManager,
    locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    withdrawn: Arc<Mutex<HashSet<String>>>,
}

impl PtySubmitter {
    pub fn new(pty: PtyManager) -> Self {
        Self {
            pty,
            locks: Arc::new(Mutex::new(HashMap::new())),
            withdrawn: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn lock_for(&self, pty_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut guard = self.locks.lock().unwrap();
        guard
            .entry(pty_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub fn withdraw(&self, ref_id: &str) -> bool {
        if ref_id.is_empty() {
            return false;
        }
        self.withdrawn.lock().unwrap().insert(ref_id.to_string());
        true
    }

    fn is_withdrawn(&self, ref_id: Option<&str>) -> bool {
        ref_id
            .map(|r| self.withdrawn.lock().unwrap().contains(r))
            .unwrap_or(false)
    }

    pub async fn submit<F>(
        &self,
        req: PtySubmitRequest,
        on_began: F,
    ) -> Result<SubmitResult, PtyError>
    where
        F: FnOnce(&PtySubmitRequest) + Send,
    {
        let chain_lock = self.lock_for(&req.pty_id);
        let _permit = chain_lock.lock().await;

        let started = std::time::Instant::now();
        if self.is_withdrawn(req.ref_id.as_deref()) {
            if let Some(ref r) = req.ref_id {
                self.withdrawn.lock().unwrap().remove(r);
            }
            return Ok(SubmitResult {
                ok: false,
                outcome: "held".to_string(),
                proof: "none".to_string(),
                presses: 0,
                ms: 0,
                reason: Some("withdrawn".to_string()),
                chars: req.text.len(),
            });
        }

        if self.pty.info(&req.pty_id).is_none() {
            return Err(PtyError::NotFound(req.pty_id));
        }

        if !req.press_only && req.provider != "custom" {
            let wait_deadline = std::time::Instant::now() + std::time::Duration::from_millis(6000);
            loop {
                let Some(info) = self.pty.info(&req.pty_id) else {
                    return Err(PtyError::NotFound(req.pty_id));
                };
                let now_epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                if info.has_output && now_epoch.saturating_sub(info.started_at) >= 1800 {
                    break;
                }
                if std::time::Instant::now() >= wait_deadline {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        on_began(&req);

        if !req.press_only && !req.text.is_empty() {
            let use_paste = (req.provider == "codex" || req.text.contains('\n'))
                && req.provider != "antigravity";
            let payload = if use_paste {
                format!("\x1b[200~{}\x1b[201~", req.text.replace('\r', ""))
            } else {
                req.text.clone()
            };
            self.pty.write(&req.pty_id, &payload)?;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        self.pty.write(&req.pty_id, "\r")?;

        if let Some(ms) = req.settle_ms
            && ms > 0
        {
            tokio::time::sleep(std::time::Duration::from_millis(ms.min(5000))).await;
        }

        if let Some(ref r) = req.ref_id {
            self.withdrawn.lock().unwrap().remove(r);
        }

        Ok(SubmitResult {
            ok: true,
            outcome: "submitted".to_string(),
            proof: "none".to_string(),
            presses: 1,
            ms: started.elapsed().as_millis() as u64,
            reason: None,
            chars: req.text.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::SpawnOptions;

    #[tokio::test]
    async fn withdraw_prevents_typing_into_pty() {
        let mgr = PtyManager::new();
        let sub = PtySubmitter::new(mgr.clone());
        let res = mgr.spawn(SpawnOptions {
            id: "sub-test".to_string(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "cat".to_string()],
            cols: Some(80),
            rows: Some(24),
            env: HashMap::new(),
            shell_script: None,
            provider: None,
            resume: None,
            hive: None,
        });
        assert!(res.ok);

        assert!(sub.withdraw("msg-1"));
        let out = sub
            .submit(
                PtySubmitRequest {
                    pty_id: "sub-test".to_string(),
                    agent_id: None,
                    provider: "claude".to_string(),
                    text: "secret".to_string(),
                    source: "queue".to_string(),
                    settle_ms: None,
                    press_only: false,
                    ref_id: Some("msg-1".to_string()),
                },
                |_| panic!("began should not fire for withdrawn request"),
            )
            .await
            .unwrap();
        assert_eq!(out.outcome, "held");
        assert_eq!(out.reason.as_deref(), Some("withdrawn"));
        mgr.kill("sub-test").ok();
    }
}
