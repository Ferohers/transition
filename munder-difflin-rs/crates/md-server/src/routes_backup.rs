//! Curated Backup Management (create, list, download, import, inspect, delete,
//! dirty-folder check & clean restore with safe agent/system shutdown).

use crate::state::AppState;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post},
};
use base64::Engine as _;
use md_core::config::{ConfigStore, expand_tilde};
use md_db::PersistStore;
use md_hive::{
    manager::HiveManager,
    roster::RosterStore,
    switch_clear::{AskedCompact, CodexSizeClear, SwitchClear},
    wake_filing::WakeFiling,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

const DATA_FILES_TO_COPY: &[&str] = &[
    "config.json",
    "harness.db",
    "integrations.json",
    "webhook-endpoints.json",
    "trigger-history.json",
    "workers-history.json",
];

const DATA_DIRS_TO_COPY: &[&str] = &[
    "knowledge",
    "agent-templates",
    "office-themes",
    "code-themes",
];

const DATA_CLEAN_PATHS: &[&str] = &[
    "config.json",
    "harness.db",
    "harness.db-wal",
    "harness.db-shm",
    "integrations.json",
    "webhook-endpoints.json",
    "trigger-history.json",
    "workers-history.json",
    "knowledge",
    "agent-templates",
    "office-themes",
    "code-themes",
];

const HIVE_EXCLUDED_ROOT_ENTRIES: &[&str] = &[
    "worktrees",
    "roster-backups",
    "bin",
    "fleet.json",
    "log.jsonl",
    "log.jsonl.1",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupManifest {
    pub version: u32,
    pub app: String,
    pub created_at: String,
    pub created_at_ms: u64,
    pub harness_home: Option<String>,
    pub god_name: Option<String>,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub name: String,
    pub size_bytes: u64,
    pub created_at_ms: u64,
    pub manifest: Option<BackupManifest>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirtyReport {
    pub dirty: bool,
    pub entries: Vec<String>,
    pub total_count: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBackupReq {
    #[serde(default)]
    pub stop_first: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportBackupReq {
    pub name: Option<String>,
    pub data_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectBackupReq {
    pub name: String,
    pub target_home: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreBackupReq {
    pub name: String,
    #[serde(default)]
    pub clean_dirty: bool,
    pub target_home: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBackupDirReq {
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirWriteCheck {
    pub path: String,
    pub exists: bool,
    pub writable: bool,
    pub is_custom: bool,
    pub error: Option<String>,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn tar_bin() -> &'static str {
    if Path::new("/usr/bin/tar").is_file() {
        "/usr/bin/tar"
    } else if Path::new("/bin/tar").is_file() {
        "/bin/tar"
    } else {
        "tar"
    }
}

fn default_backups_dir(state: &AppState) -> PathBuf {
    state.data_dir.join("backups")
}

fn backups_dir_with_flag(state: &AppState) -> (PathBuf, bool) {
    let custom = {
        let mut store = state.config.lock().unwrap();
        store
            .read()
            .extra
            .get("backupDir")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(expand_tilde)
    };
    match custom {
        Some(p) => (p, true),
        None => (default_backups_dir(state), false),
    }
}

fn backups_dir(state: &AppState) -> PathBuf {
    backups_dir_with_flag(state).0
}

fn check_dir_writable(dir: &Path, is_custom: bool) -> DirWriteCheck {
    let path_str = dir.to_string_lossy().into_owned();
    if dir.as_os_str().is_empty() || dir == Path::new("/") {
        return DirWriteCheck {
            path: path_str,
            exists: dir.exists(),
            writable: false,
            is_custom,
            error: Some("Choose a dedicated backup directory, not filesystem root.".to_string()),
        };
    }
    if dir.exists() && !dir.is_dir() {
        return DirWriteCheck {
            path: path_str,
            exists: true,
            writable: false,
            is_custom,
            error: Some("Path exists and is a file, not a directory.".to_string()),
        };
    }
    if let Err(err) = fs::create_dir_all(dir) {
        return DirWriteCheck {
            path: path_str,
            exists: false,
            writable: false,
            is_custom,
            error: Some(format!("Cannot create folder: {err}")),
        };
    }
    let probe = dir.join(format!(".md-write-test-{}", uuid::Uuid::new_v4()));
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            if let Err(err) = fs::remove_file(&probe) {
                return DirWriteCheck {
                    path: path_str,
                    exists: true,
                    writable: false,
                    is_custom,
                    error: Some(format!("Folder wrote test file but failed cleanup: {err}")),
                };
            }
            DirWriteCheck {
                path: path_str,
                exists: true,
                writable: true,
                is_custom,
                error: None,
            }
        }
        Err(err) => DirWriteCheck {
            path: path_str,
            exists: true,
            writable: false,
            is_custom,
            error: Some(format!("Folder is not writable: {err}")),
        },
    }
}

fn is_safe_backup_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && name.ends_with(".tar.gz")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn sanitize_backup_filename(raw: Option<&str>) -> String {
    let base = raw
        .and_then(|s| Path::new(s).file_name().and_then(|n| n.to_str()))
        .unwrap_or("")
        .trim();
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    if cleaned.ends_with(".tar.gz") && is_safe_backup_name(&cleaned) {
        cleaned
    } else {
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        format!("duman-backup-imported-{stamp}.tar.gz")
    }
}

fn inspect_dir_dirty(dir: &Path) -> DirtyReport {
    if !dir.exists() {
        return DirtyReport {
            dirty: false,
            entries: Vec::new(),
            total_count: 0,
        };
    }
    let mut entries = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            if name == ".DS_Store" {
                continue;
            }
            entries.push(name);
        }
    }
    entries.sort();
    let total_count = entries.len();
    entries.truncate(20);
    DirtyReport {
        dirty: total_count > 0,
        entries,
        total_count,
    }
}

fn copy_dir_filtered(
    src: &Path,
    dst: &Path,
    skip_entry_names: &[&str],
    skip_broker_files: bool,
) -> std::io::Result<()> {
    if !src.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == ".DS_Store" || name_str == ".palace-stage" {
            continue;
        }
        if skip_entry_names.iter().any(|s| *s == name_str.as_ref()) {
            continue;
        }
        if skip_broker_files && name_str == ".broker" {
            continue;
        }
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let meta = fs::symlink_metadata(&src_path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            copy_dir_filtered(&src_path, &dst_path, &[], skip_broker_files)?;
        } else if meta.is_file() {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

fn clean_dir_contents(dir: &Path) -> std::io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let meta = fs::symlink_metadata(&p)?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            fs::remove_dir_all(&p)?;
        } else {
            fs::remove_file(&p)?;
        }
    }
    Ok(())
}

fn verify_tar_safe(archive: &Path) -> Result<(), String> {
    let verbose = Command::new(tar_bin())
        .arg("-tvzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("tar inspect failed: {e}"))?;
    if !verbose.status.success() {
        let err = String::from_utf8_lossy(&verbose.stderr);
        return Err(format!("invalid tar.gz archive: {}", err.trim()));
    }
    for line in String::from_utf8_lossy(&verbose.stdout).lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('l') || trimmed.starts_with('h') {
            return Err("archive contains disallowed symlinks or hardlinks".to_string());
        }
    }

    let list = Command::new(tar_bin())
        .arg("-tzf")
        .arg(archive)
        .output()
        .map_err(|e| format!("tar list failed: {e}"))?;
    if !list.status.success() {
        let err = String::from_utf8_lossy(&list.stderr);
        return Err(format!("invalid tar.gz archive: {}", err.trim()));
    }
    let mut saw_manifest = false;
    for line in String::from_utf8_lossy(&list.stdout).lines() {
        let raw = line.trim_end_matches('\r');
        if raw.is_empty() {
            continue;
        }
        let norm = raw.strip_prefix("./").unwrap_or(raw);
        if norm.starts_with('/') || norm.split('/').any(|seg| seg == "..") {
            return Err(format!("archive contains unsafe path: {raw}"));
        }
        if norm == "manifest.json" {
            saw_manifest = true;
        }
    }
    if !saw_manifest {
        return Err("archive missing manifest.json".to_string());
    }
    Ok(())
}

fn read_manifest_from_archive(archive: &Path) -> Option<BackupManifest> {
    for member in ["manifest.json", "./manifest.json"] {
        let out = Command::new(tar_bin())
            .arg("-xOzf")
            .arg(archive)
            .arg(member)
            .output()
            .ok()?;
        if out.status.success()
            && let Ok(m) = serde_json::from_slice::<BackupManifest>(&out.stdout)
        {
            return Some(m);
        }
    }
    None
}

fn list_backups_on_disk(dir: &Path) -> Vec<BackupSummary> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let name = ent.file_name().to_string_lossy().into_owned();
        if !is_safe_backup_name(&name) || !path.is_file() {
            continue;
        }
        let meta = match fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let manifest = read_manifest_from_archive(&path);
        let created_at_ms = manifest
            .as_ref()
            .map(|m| m.created_at_ms)
            .filter(|&v| v > 0)
            .unwrap_or(mtime_ms);
        out.push(BackupSummary {
            name,
            size_bytes: meta.len(),
            created_at_ms,
            manifest,
        });
    }
    out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then_with(|| b.name.cmp(&a.name)));
    out
}

fn stop_all_runtime(state: &AppState) -> usize {
    let ptys = state.pty.list();
    let live_workers = {
        let mut workers = state.workers.lock().unwrap();
        let count = workers.len();
        workers.clear();
        count
    };
    *state.hire_inflight.lock().unwrap() = None;
    state.pty.kill_all();
    for p in &ptys {
        let agent_id = p.id.strip_prefix("pty-").unwrap_or(&p.id);
        state.submitter.forget_pty(&p.id);
        state.emit(
            "hive:agentExited",
            serde_json::json!({
                "agentId": agent_id,
                "id": p.id,
                "exitCode": 0,
                "signal": Value::Null,
                "printMode": false,
            }),
        );
    }
    ptys.len().max(live_workers)
}

async fn backup_status(State(state): State<AppState>) -> Json<Value> {
    let (bdir, is_custom) = backups_dir_with_flag(&state);
    let default_dir = default_backups_dir(&state);
    let dir_check = check_dir_writable(&bdir, is_custom);
    let ptys = state.pty.list();
    let workers = state.workers.lock().unwrap().clone();
    let running_ptys: Vec<String> = ptys.iter().map(|p| p.id.clone()).collect();
    let running_count = running_ptys.len().max(workers.len());
    let home = state.harness_home();
    let target_dirty = home
        .as_deref()
        .map(inspect_dir_dirty)
        .unwrap_or(DirtyReport {
            dirty: false,
            entries: Vec::new(),
            total_count: 0,
        });
    let backups = list_backups_on_disk(&bdir);
    Json(serde_json::json!({
        "backupsDir": bdir.to_string_lossy(),
        "defaultBackupsDir": default_dir.to_string_lossy(),
        "dirCheck": dir_check,
        "harnessHome": home.map(|p| p.to_string_lossy().into_owned()),
        "runningCount": running_count,
        "runningPtys": running_ptys,
        "targetDirty": target_dirty,
        "includedSummary": "Settings (config.json), SQLite database (harness.db), knowledge base & agent templates/themes, workspace roster.json, hive agents, memories, board.md, tasks.json, registry.json, and projects/",
        "excludedSummary": "Secret keys (broker-master.key, integration-secrets.json), live PTY processes, git worktrees, runtime logs (log.jsonl*), hive/bin/, fleet.json, roster-backups/, and .broker session files",
        "backups": backups,
    }))
}

async fn backup_set_dir(
    State(state): State<AppState>,
    Json(req): Json<SetBackupDirReq>,
) -> Json<Value> {
    let raw = req.path.as_deref().map(str::trim).unwrap_or("");
    let (candidate, is_custom, stored_val) = if raw.is_empty() {
        (default_backups_dir(&state), false, Value::Null)
    } else {
        (expand_tilde(raw), true, Value::String(raw.to_string()))
    };
    let dir_check = check_dir_writable(&candidate, is_custom);
    if !dir_check.writable {
        return Json(serde_json::json!({
            "ok": false,
            "dirCheck": dir_check,
            "error": dir_check.error.clone().unwrap_or_else(|| "Folder is not writable".to_string()),
        }));
    }
    {
        let mut store = state.config.lock().unwrap();
        if let Err(err) = store.update(serde_json::json!({ "backupDir": stored_val })) {
            return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
        }
    }
    let backups = list_backups_on_disk(&candidate);
    Json(serde_json::json!({
        "ok": true,
        "backupsDir": candidate.to_string_lossy(),
        "dirCheck": dir_check,
        "backups": backups,
    }))
}

async fn backup_create(
    State(state): State<AppState>,
    Json(req): Json<CreateBackupReq>,
) -> Json<Value> {
    let (bdir, is_custom) = backups_dir_with_flag(&state);
    let dir_check = check_dir_writable(&bdir, is_custom);
    if !dir_check.writable {
        return Json(serde_json::json!({
            "ok": false,
            "notWritable": true,
            "dirCheck": dir_check,
            "error": dir_check.error.clone().unwrap_or_else(|| "Backup folder is not writable".to_string()),
        }));
    }
    let ptys = state.pty.list();
    let live_workers_len = state.workers.lock().unwrap().len();
    let running_count = ptys.len().max(live_workers_len);
    if running_count > 0 && !req.stop_first {
        let running_ptys: Vec<String> = ptys.iter().map(|p| p.id.clone()).collect();
        return Json(serde_json::json!({
            "ok": false,
            "needsStop": true,
            "runningCount": running_count,
            "runningPtys": running_ptys,
        }));
    }

    let resume_ptys: Vec<String> = if running_count > 0 {
        ptys.iter()
            .map(|p| p.id.clone())
            .filter(|id| !id.starts_with("pty-worker-"))
            .collect()
    } else {
        Vec::new()
    };
    let stopped_count = if running_count > 0 {
        if !resume_ptys.is_empty() {
            state.emit("backup:pausing", serde_json::json!({ "ptys": resume_ptys }));
        }
        let n = stop_all_runtime(&state);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        n
    } else {
        0
    };
    let resume_paused = |ptys_to_resume: &[String]| {
        if !ptys_to_resume.is_empty() {
            state.emit("backup:resumed", serde_json::json!({ "ptys": ptys_to_resume }));
            state.emit("power:resume", serde_json::json!({ "dead": ptys_to_resume }));
        }
    };

    if let Err(err) = state.db.lock().unwrap().checkpoint_truncate() {
        resume_paused(&resume_ptys);
        return Json(serde_json::json!({
            "ok": false,
            "error": format!("SQLite checkpoint failed: {err}"),
        }));
    }

    let bdir = backups_dir(&state);
    if let Err(err) = fs::create_dir_all(&bdir) {
        resume_paused(&resume_ptys);
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }

    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut file_name = format!("duman-backup-{stamp}.tar.gz");
    if bdir.join(&file_name).exists() {
        let short = &uuid::Uuid::new_v4().to_string()[..6];
        file_name = format!("duman-backup-{stamp}-{short}.tar.gz");
    }
    let stage = std::env::temp_dir().join(format!("md-backup-stage-{}", uuid::Uuid::new_v4()));
    let stage_data = stage.join("data");
    let stage_home = stage.join("home");
    if let Err(err) = fs::create_dir_all(&stage_data) {
        resume_paused(&resume_ptys);
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }

    let mut items = Vec::new();
    for f in DATA_FILES_TO_COPY {
        let src = state.data_dir.join(f);
        if src.is_file() && fs::copy(&src, stage_data.join(f)).is_ok() {
            items.push(format!("data/{f}"));
        }
    }
    for d in DATA_DIRS_TO_COPY {
        let src = state.data_dir.join(d);
        if src.is_dir() && copy_dir_filtered(&src, &stage_data.join(d), &[], false).is_ok() {
            items.push(format!("data/{d}/"));
        }
    }

    let (harness_home_str, god_name) = {
        let mut cfg = state.config.lock().unwrap();
        let c = cfg.read();
        let god = c
            .extra
            .get("godName")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        (c.harness_home, god)
    };
    if let Some(ref home_raw) = harness_home_str {
        let home_path = expand_tilde(home_raw);
        if home_path.is_dir() {
            let _ = fs::create_dir_all(&stage_home);
            let roster_src = home_path.join("roster.json");
            if roster_src.is_file() && fs::copy(&roster_src, stage_home.join("roster.json")).is_ok() {
                items.push("home/roster.json".to_string());
            }
            let hive_src = home_path.join("hive");
            if hive_src.is_dir()
                && copy_dir_filtered(
                    &hive_src,
                    &stage_home.join("hive"),
                    HIVE_EXCLUDED_ROOT_ENTRIES,
                    true,
                )
                .is_ok()
            {
                items.push("home/hive/".to_string());
            }
        }
    }

    let manifest = BackupManifest {
        version: 1,
        app: "duman-office".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        created_at_ms: now_ms(),
        harness_home: harness_home_str,
        god_name,
        items,
    };
    let manifest_json = match serde_json::to_string_pretty(&manifest) {
        Ok(s) => s,
        Err(e) => {
            let _ = fs::remove_dir_all(&stage);
            resume_paused(&resume_ptys);
            return Json(serde_json::json!({ "ok": false, "error": e.to_string() }));
        }
    };
    if let Err(err) = fs::write(stage.join("manifest.json"), manifest_json) {
        let _ = fs::remove_dir_all(&stage);
        resume_paused(&resume_ptys);
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }

    let tmp_archive = bdir.join(format!(".tmp-{file_name}"));
    let final_archive = bdir.join(&file_name);
    let tar_out = Command::new(tar_bin())
        .arg("-czf")
        .arg(&tmp_archive)
        .arg("-C")
        .arg(&stage)
        .arg(".")
        .output();
    let _ = fs::remove_dir_all(&stage);
    resume_paused(&resume_ptys);

    match tar_out {
        Ok(out) if out.status.success() => {
            if let Err(err) = fs::rename(&tmp_archive, &final_archive) {
                let _ = fs::remove_file(&tmp_archive);
                return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
            }
            let size_bytes = fs::metadata(&final_archive).map(|m| m.len()).unwrap_or(0);
            Json(serde_json::json!({
                "ok": true,
                "stoppedAgents": stopped_count,
                "backup": BackupSummary {
                    name: file_name,
                    size_bytes,
                    created_at_ms: manifest.created_at_ms,
                    manifest: Some(manifest),
                }
            }))
        }
        Ok(out) => {
            let _ = fs::remove_file(&tmp_archive);
            let err = String::from_utf8_lossy(&out.stderr);
            Json(serde_json::json!({ "ok": false, "error": format!("tar failed: {}", err.trim()) }))
        }
        Err(err) => {
            let _ = fs::remove_file(&tmp_archive);
            Json(serde_json::json!({ "ok": false, "error": format!("tar failed: {err}") }))
        }
    }
}

async fn backup_download(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Result<Response, StatusCode> {
    if !is_safe_backup_name(&name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = backups_dir(&state).join(&name);
    let bytes = fs::read(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/gzip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\""),
        )
        .body(Body::from(bytes))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn backup_delete(
    State(state): State<AppState>,
    AxumPath(name): AxumPath<String>,
) -> Json<Value> {
    if !is_safe_backup_name(&name) {
        return Json(serde_json::json!({ "ok": false, "error": "invalid backup name" }));
    }
    let path = backups_dir(&state).join(&name);
    if !path.is_file() {
        return Json(serde_json::json!({ "ok": false, "error": "backup not found" }));
    }
    match fs::remove_file(&path) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(err) => Json(serde_json::json!({ "ok": false, "error": err.to_string() })),
    }
}

async fn backup_import(
    State(state): State<AppState>,
    Json(req): Json<ImportBackupReq>,
) -> Json<Value> {
    let raw_b64 = req.data_base64.trim();
    let b64_payload = raw_b64
        .split_once("base64,")
        .map(|(_, after)| after)
        .unwrap_or(raw_b64);
    let bytes = match base64::engine::general_purpose::STANDARD.decode(b64_payload) {
        Ok(b) => b,
        Err(err) => {
            return Json(serde_json::json!({ "ok": false, "error": format!("invalid base64: {err}") }));
        }
    };
    let bdir = backups_dir(&state);
    if let Err(err) = fs::create_dir_all(&bdir) {
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }
    let mut name = sanitize_backup_filename(req.name.as_deref());
    if bdir.join(&name).exists() {
        let stem = name.trim_end_matches(".tar.gz");
        let short = &uuid::Uuid::new_v4().to_string()[..6];
        name = format!("{stem}-{short}.tar.gz");
    }
    let tmp = bdir.join(format!(".tmp-import-{}", uuid::Uuid::new_v4()));
    if let Err(err) = fs::write(&tmp, &bytes) {
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }
    if let Err(err) = verify_tar_safe(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Json(serde_json::json!({ "ok": false, "error": err }));
    }
    let dest = bdir.join(&name);
    if let Err(err) = fs::rename(&tmp, &dest) {
        let _ = fs::remove_file(&tmp);
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }
    let manifest = read_manifest_from_archive(&dest);
    let created_at_ms = manifest
        .as_ref()
        .map(|m| m.created_at_ms)
        .unwrap_or_else(now_ms);
    Json(serde_json::json!({
        "ok": true,
        "backup": BackupSummary {
            name,
            size_bytes: bytes.len() as u64,
            created_at_ms,
            manifest,
        }
    }))
}

fn resolve_restore_target(
    state: &AppState,
    req_target: Option<&str>,
    manifest: Option<&BackupManifest>,
) -> Option<PathBuf> {
    if let Some(raw) = req_target.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(expand_tilde(raw));
    }
    if let Some(h) = state.harness_home() {
        return Some(h);
    }
    manifest
        .and_then(|m| m.harness_home.as_deref())
        .map(expand_tilde)
}

async fn backup_inspect(
    State(state): State<AppState>,
    Json(req): Json<InspectBackupReq>,
) -> Json<Value> {
    if !is_safe_backup_name(&req.name) {
        return Json(serde_json::json!({ "ok": false, "error": "invalid backup name" }));
    }
    let archive = backups_dir(&state).join(&req.name);
    if !archive.is_file() {
        return Json(serde_json::json!({ "ok": false, "error": "backup not found" }));
    }
    if let Err(err) = verify_tar_safe(&archive) {
        return Json(serde_json::json!({ "ok": false, "error": err }));
    }
    let manifest = read_manifest_from_archive(&archive);
    let target = resolve_restore_target(&state, req.target_home.as_deref(), manifest.as_ref());
    let target_dirty = target
        .as_deref()
        .map(inspect_dir_dirty)
        .unwrap_or(DirtyReport {
            dirty: false,
            entries: Vec::new(),
            total_count: 0,
        });
    let running_count = state
        .pty
        .list()
        .len()
        .max(state.workers.lock().unwrap().len());
    Json(serde_json::json!({
        "ok": true,
        "manifest": manifest,
        "targetHome": target.map(|p| p.to_string_lossy().into_owned()),
        "targetDirty": target_dirty,
        "runningCount": running_count,
    }))
}

async fn backup_restore(
    State(state): State<AppState>,
    Json(req): Json<RestoreBackupReq>,
) -> Json<Value> {
    if !is_safe_backup_name(&req.name) {
        return Json(serde_json::json!({ "ok": false, "error": "invalid backup name" }));
    }
    let archive = backups_dir(&state).join(&req.name);
    if !archive.is_file() {
        return Json(serde_json::json!({ "ok": false, "error": "backup not found" }));
    }
    if let Err(err) = verify_tar_safe(&archive) {
        return Json(serde_json::json!({ "ok": false, "error": err }));
    }
    let manifest = read_manifest_from_archive(&archive);
    let target = resolve_restore_target(&state, req.target_home.as_deref(), manifest.as_ref());
    if let Some(ref p) = target
        && (p.as_os_str().is_empty() || p == Path::new("/"))
    {
        return Json(serde_json::json!({ "ok": false, "error": "unsafe target workspace path" }));
    }

    let running_count = state
        .pty
        .list()
        .len()
        .max(state.workers.lock().unwrap().len());
    let target_dirty = target
        .as_deref()
        .map(inspect_dir_dirty)
        .unwrap_or(DirtyReport {
            dirty: false,
            entries: Vec::new(),
            total_count: 0,
        });
    if (running_count > 0 || target_dirty.dirty) && !req.clean_dirty {
        return Json(serde_json::json!({
            "ok": false,
            "needsClean": true,
            "runningCount": running_count,
            "targetHome": target.map(|p| p.to_string_lossy().into_owned()),
            "targetDirty": target_dirty,
        }));
    }

    // 1. Extract into verified temp staging directory before touching live disk state.
    let stage = std::env::temp_dir().join(format!("md-restore-stage-{}", uuid::Uuid::new_v4()));
    if let Err(err) = fs::create_dir_all(&stage) {
        return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
    }
    let tar_out = Command::new(tar_bin())
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage)
        .output();
    match tar_out {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let _ = fs::remove_dir_all(&stage);
            let err = String::from_utf8_lossy(&out.stderr);
            return Json(serde_json::json!({ "ok": false, "error": format!("tar extract failed: {}", err.trim()) }));
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&stage);
            return Json(serde_json::json!({ "ok": false, "error": format!("tar extract failed: {err}") }));
        }
    }

    // 2. Safely stop all live agents & reset in-memory runtime state.
    let _ = stop_all_runtime(&state);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    state.workers_history.lock().unwrap().clear();
    state.ended_agents.lock().unwrap().clear();
    state.integrations_activity.lock().unwrap().clear();
    state.control.lock().unwrap().clear_all();
    state.breaker.lock().unwrap().clear_all();
    state.hooks.clear_all();
    *state.wake_filing.lock().unwrap() = WakeFiling::new();
    *state.switch_clear.lock().unwrap() = SwitchClear::new();
    *state.codex_size.lock().unwrap() = CodexSizeClear::new();
    *state.asked_compact.lock().unwrap() = AskedCompact::new();
    state
        .hire_runner_ready
        .store(false, Ordering::Relaxed);
    {
        let mut ide = state.ide.lock().unwrap();
        ide.unsaved_count = 0;
        ide.open_files.clear();
    }

    // 3. Close live SQLite connection temporarily so WAL/SHM swap has no open handles.
    if let Ok(mem_db) = PersistStore::open_in_memory() {
        *state.db.lock().unwrap() = mem_db;
    }

    // 4. Clean non-secret files in data_dir while keeping backups/, broker-master.key, integration-secrets.json.
    for rel in DATA_CLEAN_PATHS {
        let p = state.data_dir.join(rel);
        if p.is_dir() {
            let _ = fs::remove_dir_all(&p);
        } else if p.exists() {
            let _ = fs::remove_file(&p);
        }
    }

    let stage_data = stage.join("data");
    if stage_data.is_dir() {
        for f in DATA_FILES_TO_COPY {
            let src = stage_data.join(f);
            if src.is_file() {
                let _ = fs::copy(&src, state.data_dir.join(f));
            }
        }
        for d in DATA_DIRS_TO_COPY {
            let src = stage_data.join(d);
            if src.is_dir() {
                let _ = copy_dir_filtered(&src, &state.data_dir.join(d), &[], false);
            }
        }
    }

    // 5. Clean leftover files in target workspace folder and restore home/ contents.
    if let Some(ref home_dir) = target {
        if let Err(err) = fs::create_dir_all(home_dir) {
            let _ = fs::remove_dir_all(&stage);
            return Json(serde_json::json!({ "ok": false, "error": err.to_string() }));
        }
        if let Err(err) = clean_dir_contents(home_dir) {
            let _ = fs::remove_dir_all(&stage);
            return Json(serde_json::json!({ "ok": false, "error": format!("failed cleaning folder: {err}") }));
        }
        let stage_home = stage.join("home");
        if stage_home.is_dir() {
            let roster_src = stage_home.join("roster.json");
            if roster_src.is_file() {
                let _ = fs::copy(&roster_src, home_dir.join("roster.json"));
            }
            let hive_src = stage_home.join("hive");
            if hive_src.is_dir() {
                let _ = copy_dir_filtered(&hive_src, &home_dir.join("hive"), &[], true);
            }
        }
    }
    let _ = fs::remove_dir_all(&stage);

    // Preserve local machine backupDir if already configured so cross-OS restores keep local backup storage working.
    let preserved_backup_dir = backups_dir_with_flag(&state);

    // 6. Reload all persisted stores & sync restored roster/fleet.
    let restored_cfg = {
        let mut store = state.config.lock().unwrap();
        *store = ConfigStore::new(&state.data_dir);
        let mut cfg_patch = serde_json::Map::new();
        if let Some(ref home_dir) = target {
            cfg_patch.insert(
                "harnessHome".to_string(),
                Value::String(home_dir.to_string_lossy().into_owned()),
            );
        }
        if preserved_backup_dir.1 {
            cfg_patch.insert(
                "backupDir".to_string(),
                Value::String(preserved_backup_dir.0.to_string_lossy().into_owned()),
            );
        } else {
            cfg_patch.insert("backupDir".to_string(), Value::Null);
        }
        if !cfg_patch.is_empty() {
            let _ = store.update(Value::Object(cfg_patch));
        }
        store.read()
    };
    if let Ok(reopened_db) = PersistStore::open(state.data_dir.join("harness.db")) {
        *state.db.lock().unwrap() = reopened_db;
    }
    *state.webhooks.lock().unwrap() =
        md_integrations::webhooks::WebhookRegistry::open(Some(&state.data_dir));
    *state.broker.lock().unwrap() =
        md_integrations::secrets::IntegrationBroker::open(&state.data_dir);
    let workers_hist: Vec<Value> = fs::read_to_string(state.data_dir.join("workers-history.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    *state.workers_history.lock().unwrap() = workers_hist;

    state.sync_config_runtime(&restored_cfg);
    *state.roster.lock().unwrap() = RosterStore::new();
    *state.hive.lock().unwrap() = HiveManager::new(None);
    state.sync_hive_home();
    if let Some(snap) = state
        .harness_home()
        .and_then(|h| state.roster.lock().unwrap().read(Some(&h)))
    {
        let _ = state
            .hive
            .lock()
            .unwrap()
            .sync_roster(&snap.agents, &snap.archived);
        crate::hive_bootstrap::sync_fleet_json(&state);
    }

    let epoch = state.reset_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(val) = serde_json::to_value(&restored_cfg) {
        state.emit("config:changed", val);
    }
    state.emit("app:reset", serde_json::json!({ "epoch": epoch, "restored": true }));

    Json(serde_json::json!({
        "ok": true,
        "epoch": epoch,
        "targetHome": target.map(|p| p.to_string_lossy().into_owned()),
    }))
}

pub fn backup_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/backups/status", get(backup_status))
        .route("/api/v1/backups/set-dir", post(backup_set_dir))
        .route("/api/v1/backups/create", post(backup_create))
        .route(
            "/api/v1/backups/import",
            post(backup_import).layer(DefaultBodyLimit::max(100 * 1024 * 1024)),
        )
        .route("/api/v1/backups/inspect", post(backup_inspect))
        .route("/api/v1/backups/restore", post(backup_restore))
        .route(
            "/api/v1/backups/{name}",
            axum::routing::delete(backup_delete),
        )
        .route("/api/v1/backups/{name}/download", get(backup_download))
}
