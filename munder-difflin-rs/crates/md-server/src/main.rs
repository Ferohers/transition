mod hive_bootstrap;
mod routes_agent_file;
mod routes_catalog;
mod routes_fs_git;
mod routes_hive;
mod routes_integrations;
mod routes_silenced;
mod routes_usage_memory_kg;
mod security;
mod state;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use md_core::{
    AppInfo,
    config::{ConfigStore, HarnessConfig, default_data_dir},
    damaged::IntegrityIssue,
    floor_box::{HomeBox, HomeBoxInput, HomeStageKind, OFFICE_FLOOR_H, OFFICE_FLOOR_W, home_box_of},
};
use md_db::{CommandHistoryEntry, PersistStore};
use md_hive::{
    breaker::CircuitBreaker,
    control::ControlRegistry,
    hooks::HookEngine,
    manager::HiveManager,
    roster::{RosterSnapshot, RosterStore, RosterWriteResult},
    switch_clear::{
        CodexSizeClear, SwitchClear, TypedKind, oldest_waking_work, switch_clear_command,
        typed_kind,
    },
    wake_filing::WakeFiling,
};
use md_pty::{
    manager::{PtyEvent, PtyListEntry, PtyManager, SpawnOptions, SpawnResult},
    submitter::{PtySubmitRequest, PtySubmitter, TerminalProbeReply, TerminalProbes},
};
use serde::Deserialize;
use state::{AppState, EndedAgentRecord, IdeState, ServerEvent};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tower_http::services::ServeDir;

fn spawn_pty_event_forwarder(
    pty: &PtyManager,
    workers: Arc<Mutex<Vec<state::WorkerRecord>>>,
    wake_filing: Arc<Mutex<WakeFiling>>,
    switch_clear: Arc<Mutex<SwitchClear>>,
    codex_size: Arc<Mutex<CodexSizeClear>>,
    ended_agents: Arc<Mutex<HashMap<String, EndedAgentRecord>>>,
    events_tx: broadcast::Sender<ServerEvent>,
) {
    let mut rx = pty.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(PtyEvent::Data { id, data }) => {
                    let _ = events_tx.send(ServerEvent {
                        channel: format!("pty:data:{id}"),
                        payload: serde_json::json!(data),
                    });
                }
                Ok(PtyEvent::Exit {
                    id,
                    exit_code,
                    signal,
                    tail,
                    started_at,
                }) => {
                    let is_worker = workers.lock().unwrap().iter().any(|w| w.pty_id == id);
                    let agent_id = workers
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|w| w.pty_id == id)
                        .map(|w| w.worker_id.clone())
                        .unwrap_or_else(|| id.strip_prefix("pty-").unwrap_or(&id).to_string());
                    wake_filing.lock().unwrap().forget(&agent_id);
                    switch_clear.lock().unwrap().forget(&agent_id);
                    codex_size.lock().unwrap().forget(&agent_id);
                    if !is_worker {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        ended_agents.lock().unwrap().insert(
                            agent_id.clone(),
                            EndedAgentRecord {
                                agent_id: agent_id.clone(),
                                exit_code,
                                signal: signal.clone(),
                                print_mode: false,
                                at: now,
                            },
                        );
                    }
                    let payload = serde_json::json!({
                        "exitCode": exit_code,
                        "signal": signal,
                        "tail": tail,
                        "startedAt": started_at,
                    });
                    let _ = events_tx.send(ServerEvent {
                        channel: format!("pty:exit:{id}"),
                        payload,
                    });
                    let _ = events_tx.send(ServerEvent {
                        channel: "hive:agentExited".to_string(),
                        payload: serde_json::json!({
                            "agentId": agent_id,
                            "id": id,
                            "exitCode": exit_code,
                            "signal": signal,
                            "printMode": false
                        }),
                    });
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[derive(Debug, Deserialize)]
struct FloorBoxQuery {
    home_w: Option<f64>,
    stage_h: Option<f64>,
    inner: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnsureHomeReq {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryAddReq {
    agent_id: String,
    cwd: Option<String>,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryListQuery {
    agent_id: Option<String>,
    limit: Option<usize>,
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PtyWriteReq {
    id: String,
    data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PtyResizeReq {
    id: String,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PtyIdReq {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PtyWithdrawReq {
    #[serde(rename = "ref")]
    ref_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexFolderReq {
    folder: String,
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "ok": true,
        "service": "munder-difflin-rs",
        "version": md_core::APP_VERSION
    }))
}

async fn reset_all(State(state): State<AppState>) -> Json<serde_json::Value> {
    let data_dir = state.data_dir.clone();

    // Clear in-memory config, SQLite tables, and registries.
    {
        let mut store = state.config.lock().unwrap();
        *store = md_core::config::ConfigStore::new(&data_dir);
    }
    let _ = state.db.lock().unwrap().clear_all();
    state.secrets.lock().unwrap().clear_all();
    state.broker.lock().unwrap().clear_all();
    state.webhooks.lock().unwrap().clear_all();
    state.file_share.lock().unwrap().stop_all();
    state.workers.lock().unwrap().clear();
    state.workers_history.lock().unwrap().clear();
    state.ended_agents.lock().unwrap().clear();
    state.integrations_activity.lock().unwrap().clear();
    state.control.lock().unwrap().clear_all();
    state.breaker.lock().unwrap().clear_all();
    state.sync_hive_home();
    {
        let mut ide = state.ide.lock().unwrap();
        ide.unsaved_count = 0;
        ide.open_files.clear();
    }

    // Wipe config/state files in the data dir while preserving the encryption master key
    // and the live SQLite database files (already cleared via clear_all()).
    if let Ok(entries) = std::fs::read_dir(&data_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str == "broker-master.key" || name_str.starts_with("harness.db") {
                continue;
            }
            let path = entry.path();
            if path.is_file() {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    Json(serde_json::json!({ "ok": true }))
}

async fn app_info() -> Json<AppInfo> {
    Json(AppInfo::default())
}

async fn floor_box(Query(q): Query<FloorBoxQuery>) -> Json<HomeBox> {
    let out = home_box_of(HomeBoxInput {
        kind: HomeStageKind::Floor,
        home_w: q.home_w.unwrap_or(1440.0),
        stage_h: q.stage_h.unwrap_or(900.0),
        inner: q.inner.unwrap_or(OFFICE_FLOOR_W / OFFICE_FLOOR_H),
        receipt: false,
    });
    Json(out)
}

async fn config_get(State(state): State<AppState>) -> Json<HarnessConfig> {
    let mut store = state.config.lock().unwrap();
    Json(store.read())
}

async fn config_update(
    State(state): State<AppState>,
    Json(patch): Json<serde_json::Value>,
) -> Result<Json<HarnessConfig>, (StatusCode, Json<serde_json::Value>)> {
    let next = {
        let mut store = state.config.lock().unwrap();
        store.update(patch)
    };
    match next {
        Ok(cfg) => {
            state.sync_hive_home();
            state.sync_config_runtime(&cfg);
            crate::hive_bootstrap::sync_fleet_json(&state);
            if let Ok(val) = serde_json::to_value(&cfg) {
                state.emit("config:changed", val);
            }
            Ok(Json(cfg))
        }
        Err(e) => Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
        )),
    }
}

async fn config_ensure_home(
    State(state): State<AppState>,
    Json(req): Json<EnsureHomeReq>,
) -> Json<serde_json::Value> {
    let store = state.config.lock().unwrap();
    match store.ensure_harness_home(&req.path) {
        Ok(path) => Json(serde_json::json!({ "ok": true, "path": path })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn integrity_issues(State(state): State<AppState>) -> Json<Vec<IntegrityIssue>> {
    let mut issues = state.config.lock().unwrap().integrity_issues();
    state.sync_hive_home();
    issues.extend(state.hive.lock().unwrap().integrity_issues());
    Json(issues)
}

async fn roster_get(State(state): State<AppState>) -> Json<Option<RosterSnapshot>> {
    let home = state.harness_home();
    let store = state.roster.lock().unwrap();
    Json(store.read(home.as_deref()))
}

async fn roster_put(
    State(state): State<AppState>,
    Json(snap): Json<RosterSnapshot>,
) -> Json<RosterWriteResult> {
    let home = state.harness_home();
    let res = {
        let mut store = state.roster.lock().unwrap();
        store.write(home.as_deref(), &snap)
    };
    if res.ok {
        state.sync_hive_home();
        let _ = state
            .hive
            .lock()
            .unwrap()
            .sync_roster(&snap.agents, &snap.archived);
        crate::hive_bootstrap::sync_fleet_json(&state);
        state.emit(
            "roster:updated",
            serde_json::json!({ "count": snap.entry_count() }),
        );
    }
    Json(res)
}

async fn history_add(
    State(state): State<AppState>,
    Json(req): Json<HistoryAddReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let db = state.db.lock().unwrap();
    db.add_history(&req.agent_id, req.cwd.as_deref(), &req.text)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn history_list(
    State(state): State<AppState>,
    Query(q): Query<HistoryListQuery>,
) -> Result<Json<Vec<CommandHistoryEntry>>, StatusCode> {
    let db = state.db.lock().unwrap();
    let limit = q.limit.unwrap_or(100);
    let rows = if let Some(ref query) = q.q {
        db.search_history(query, limit)
    } else {
        db.list_history(q.agent_id.as_deref(), limit)
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(rows))
}

async fn pty_spawn(
    State(state): State<AppState>,
    Json(mut opts): Json<SpawnOptions>,
) -> Json<SpawnResult> {
    let hive_meta = opts.hive.as_ref().and_then(|v| v.as_object());
    let agent_id = hive_meta
        .and_then(|m| m.get("id").and_then(|v| v.as_str()))
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            opts.id
                .strip_prefix("pty-")
                .unwrap_or(&opts.id)
                .to_string()
        });
    let name = hive_meta
        .and_then(|m| m.get("name").and_then(|v| v.as_str()))
        .map(ToString::to_string)
        .unwrap_or_else(|| agent_id.clone());
    let provider = opts
        .provider
        .clone()
        .or_else(|| {
            hive_meta
                .and_then(|m| m.get("provider").and_then(|v| v.as_str()))
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "claude".to_string());
    let is_god = hive_meta
        .and_then(|m| m.get("isGod").and_then(|v| v.as_bool()))
        .unwrap_or(agent_id == "god");
    let role = hive_meta
        .and_then(|m| m.get("role").and_then(|v| v.as_str()))
        .map(ToString::to_string);

    // On web, allow creating new workspace subfolders inside configured `harnessHome`
    // (e.g. `<harnessHome>/Projects`) when adding an agent.
    if let Some(home) = state.harness_home() {
        let cwd_path = md_core::config::expand_tilde(&opts.cwd);
        let canon_h = std::fs::canonicalize(&home).unwrap_or_else(|_| home.clone());
        if !cwd_path.is_dir() && (cwd_path.starts_with(&home) || cwd_path.starts_with(&canon_h)) {
            let _ = std::fs::create_dir_all(&cwd_path);
        }
    }

    let seed_prompt = crate::hive_bootstrap::prepare_agent_spawn(
        &state,
        &mut opts,
        &agent_id,
        &name,
        &provider,
        is_god,
        role.as_deref(),
    );
    let mut res = state.pty.spawn(opts);
    if res.ok {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        state.ended_agents.lock().unwrap().remove(&agent_id);
        state.switch_clear.lock().unwrap().note_spawn(&agent_id, now);
        state.codex_size.lock().unwrap().forget(&agent_id);
        crate::hive_bootstrap::sync_fleet_json(&state);
        res.seed_prompt = seed_prompt;
        res.resumed = Some(false);
    }
    Json(res)
}

async fn pty_write(
    State(state): State<AppState>,
    Json(req): Json<PtyWriteReq>,
) -> Json<serde_json::Value> {
    match state.pty.write(&req.id, &req.data) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn pty_resize(
    State(state): State<AppState>,
    Json(req): Json<PtyResizeReq>,
) -> Json<serde_json::Value> {
    match state.pty.resize(&req.id, req.cols, req.rows) {
        Ok(pending) => {
            if pending {
                Json(serde_json::json!({ "ok": true, "pending": true }))
            } else {
                Json(serde_json::json!({ "ok": true }))
            }
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn pty_redraw(
    State(state): State<AppState>,
    Json(req): Json<PtyIdReq>,
) -> Json<serde_json::Value> {
    match state.pty.redraw(&req.id).await {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn pty_kill(
    State(state): State<AppState>,
    Json(req): Json<PtyIdReq>,
) -> Json<serde_json::Value> {
    match state.pty.kill(&req.id) {
        Ok(()) => Json(serde_json::json!({ "ok": true })),
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn pty_list(State(state): State<AppState>) -> Json<Vec<PtyListEntry>> {
    Json(state.pty.list())
}

async fn pty_tail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "id": id, "tail": state.pty.tail(&id) }))
}

async fn pty_sleep_screen(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<serde_json::Value> {
    // When PTY is live, `ws_pty` already replays the tail buffer; returning empty here
    // prevents double-writing ANSI screen bytes (`upstream/src/main/index.ts:4851`).
    if state.pty.info(&id).is_some() {
        return Json(serde_json::json!({ "id": id, "screen": "" }));
    }
    let agent_id = id.strip_prefix("pty-").unwrap_or(&id);
    let screen = if md_core::repo_fs::is_safe_id(agent_id)
        && let Some(home) = state.harness_home()
    {
        let p = home
            .join("hive")
            .join("agents")
            .join(agent_id)
            .join("sleep-screen.ansi");
        std::fs::read_to_string(p).unwrap_or_default()
    } else {
        String::new()
    };
    Json(serde_json::json!({ "id": id, "screen": screen }))
}

async fn pty_submit(
    State(state): State<AppState>,
    Json(req): Json<PtySubmitRequest>,
) -> Json<serde_json::Value> {
    if !req.press_only
        && req.source != "queue"
        && !routes_hive::probe_terminal_safe(&state, &req.pty_id).await
    {
        return Json(serde_json::json!({
            "ok": true,
            "sent": false,
            "result": {
                "ok": false,
                "outcome": "held",
                "proof": "none",
                "presses": 0,
                "ms": 0,
                "reason": "draft-or-menu",
                "chars": req.text.len(),
            }
        }));
    }
    let state_clone = state.clone();
    let eff_agent_id = req
        .agent_id
        .clone()
        .or_else(|| req.pty_id.strip_prefix("pty-").map(ToString::to_string));
    let provider = req.provider.clone();
    let text = req.text.clone();
    let press_only = req.press_only;
    let probe = routes_hive::AppScreenProbe::new(&state);
    let hooks = routes_hive::AppSubmitHooks(&state);
    match state
        .submitter
        .submit_with_context(req, Some(&probe), &hooks, move |r| {
            state_clone.emit("pty:began", serde_json::json!(r.pty_id));
        })
        .await
    {
        Ok(res) => {
            if let Some(ref aid) = eff_agent_id {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let to_file = state.wake_filing.lock().unwrap().note_typed(
                    aid,
                    &text,
                    &res.outcome,
                    res.ms,
                    press_only,
                    now,
                );
                if !to_file.is_empty() {
                    state.sync_hive_home();
                    let _ = state.hive.lock().unwrap().file_inbox_messages(aid, &to_file);
                }
                let kind = typed_kind(&text, switch_clear_command(&provider));
                let wake_work = if kind == TypedKind::Wake {
                    state.sync_hive_home();
                    let inbox = state.hive.lock().unwrap().inbox(aid);
                    oldest_waking_work(&inbox)
                } else {
                    None
                };
                state.switch_clear.lock().unwrap().note_typed(
                    Some(aid),
                    kind,
                    &res.outcome,
                    press_only,
                    wake_work,
                    now,
                );
                if let Some(rotated_bytes) = state.codex_size.lock().unwrap().note_typed(
                    Some(aid),
                    kind,
                    &res.outcome,
                    press_only,
                ) {
                    state.sync_hive_home();
                    state.hive.lock().unwrap().append_log(&serde_json::json!({
                        "kind": "codex-session-rotated",
                        "agentId": aid,
                        "bytes": rotated_bytes,
                    }));
                }
            }
            Json(serde_json::json!({
                "ok": true,
                "sent": res.outcome == "submitted" || res.outcome == "unconfirmed",
                "result": res,
            }))
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn pty_withdraw(
    State(state): State<AppState>,
    Json(req): Json<PtyWithdrawReq>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": state.submitter.withdraw(&req.ref_id) }))
}

async fn terminal_probe(
    State(state): State<AppState>,
    Json(req): Json<PtyIdReq>,
) -> Json<serde_json::Value> {
    let Some(info) = state.pty.info(&req.id) else {
        return Json(serde_json::json!({ "ok": false, "error": "no pty" }));
    };
    let (probe_req, rx) = state.probes.register(&req.id, info.cols, info.rows);
    let probe_id = probe_req.id.clone();
    if let Ok(val) = serde_json::to_value(&probe_req) {
        state.emit("terminal:probe", val);
    }
    let reply = state.probes.wait_with_timeout(&probe_id, rx).await;
    Json(serde_json::json!({ "ok": true, "reply": reply }))
}

async fn terminal_probe_reply(
    State(state): State<AppState>,
    Json(reply): Json<TerminalProbeReply>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": state.probes.resolve(reply) }))
}

async fn codex_allowed_folders(State(state): State<AppState>) -> Json<Vec<String>> {
    let mut store = state.config.lock().unwrap();
    let cfg = store.read();
    let folders = cfg
        .extra
        .get("codexLayerOptIns")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    Json(folders)
}

async fn codex_allow_folder(
    State(state): State<AppState>,
    Json(req): Json<CodexFolderReq>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let mut folders = {
        let mut store = state.config.lock().unwrap();
        let cfg = store.read();
        cfg.extra
            .get("codexLayerOptIns")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    if !req.folder.trim().is_empty() && !folders.contains(&req.folder) {
        folders.push(req.folder);
        let mut store = state.config.lock().unwrap();
        store
            .update(serde_json::json!({ "codexLayerOptIns": folders }))
            .map_err(|_| StatusCode::CONFLICT)?;
    }
    Ok(Json(folders))
}

async fn codex_withdraw_folder(
    State(state): State<AppState>,
    Json(req): Json<CodexFolderReq>,
) -> Result<Json<Vec<String>>, StatusCode> {
    let mut folders = {
        let mut store = state.config.lock().unwrap();
        let cfg = store.read();
        cfg.extra
            .get("codexLayerOptIns")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    folders.retain(|f| f != &req.folder);
    let mut store = state.config.lock().unwrap();
    store
        .update(serde_json::json!({ "codexLayerOptIns": folders }))
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(folders))
}

async fn ws_events(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_events(socket, state))
}

async fn handle_ws_events(mut socket: WebSocket, state: AppState) {
    let mut rx = state.events_tx.subscribe();
    loop {
        tokio::select! {
            evt = rx.recv() => {
                match evt {
                    Ok(ev) => {
                        if let Ok(text) = serde_json::to_string(&ev)
                            && socket.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Ping(bytes))) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

async fn ws_pty(
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_pty(socket, state, id))
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum PtyWsInput {
    Input { data: String },
    Resize { cols: u16, rows: u16 },
}

async fn handle_ws_pty(mut socket: WebSocket, state: AppState, id: String) {
    let mut rx = state.pty.subscribe();
    let tail = state.pty.tail(&id);
    if !tail.is_empty() {
        let msg = serde_json::json!({ "kind": "data", "id": id, "data": tail });
        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
            return;
        }
    }

    loop {
        tokio::select! {
            evt = rx.recv() => {
                match evt {
                    Ok(ev) => {
                        let matches = match &ev {
                            PtyEvent::Data { id: ev_id, .. } => ev_id == &id,
                            PtyEvent::Exit { id: ev_id, .. } => ev_id == &id,
                        };
                        if matches
                            && let Ok(text) = serde_json::to_string(&ev)
                            && socket.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(txt))) => {
                        if let Ok(parsed) = serde_json::from_str::<PtyWsInput>(&txt) {
                            match parsed {
                                PtyWsInput::Input { data } => {
                                    let _ = state.pty.write(&id, &data);
                                }
                                PtyWsInput::Resize { cols, rows } => {
                                    let _ = state.pty.resize(&id, cols, rows);
                                }
                            }
                        } else {
                            let _ = state.pty.write(&id, &txt);
                        }
                    }
                    Some(Ok(Message::Ping(bytes))) => {
                        if socket.send(Message::Pong(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

fn web_root() -> PathBuf {
    if let Ok(explicit) = std::env::var("MD_WEB_DIR") {
        let p = PathBuf::from(explicit.trim());
        if p.is_dir() {
            return p;
        }
    }
    let cwd_web = PathBuf::from("./web");
    if cwd_web.is_dir() {
        return cwd_web;
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors().skip(1).take(4) {
            let p = ancestor.join("web");
            if p.is_dir() {
                return p;
            }
        }
    }
    let manifest_web = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("web");
    if manifest_web.is_dir() {
        return manifest_web;
    }
    cwd_web
}

async fn index_html() -> Html<String> {
    let p = web_root().join("index.html");
    match tokio::fs::read_to_string(&p).await {
        Ok(html) => Html(html),
        Err(_) => Html(include_str!("../static/index.html").to_string()),
    }
}

async fn ide_html() -> Html<String> {
    let p = web_root().join("ide.html");
    match tokio::fs::read_to_string(&p).await {
        Ok(html) => Html(html),
        Err(_) => Html("<!doctype html><body>IDE bundle missing</body>".to_string()),
    }
}

async fn cth_web_js() -> impl IntoResponse {
    let p = web_root().join("cth-web.js");
    let body = tokio::fs::read_to_string(&p).await.unwrap_or_default();
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        body,
    )
}

async fn assets_styles() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/styles.css"),
    )
}

async fn assets_app_js() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../static/app.js"),
    )
}

pub fn build_router(state: AppState) -> Router {
    let assets_dir = web_root().join("assets");
    Router::new()
        .route("/", get(index_html))
        .route("/ide.html", get(ide_html))
        .route("/cth-web.js", get(cth_web_js))
        .route("/static/styles.css", get(assets_styles))
        .route("/static/app.js", get(assets_app_js))
        .nest_service("/assets", ServeDir::new(assets_dir))
        .route("/health", get(health))
        .route("/ws/events", get(ws_events))
        .route("/ws/pty/{id}", get(ws_pty))
        .route("/api/v1/info", get(app_info))
        .route("/api/v1/floor/box", get(floor_box))
        .route("/api/v1/reset", post(reset_all))
        .route("/api/v1/config", get(config_get).patch(config_update))
        .route("/api/v1/config/ensure-home", post(config_ensure_home))
        .route("/api/v1/integrity/issues", get(integrity_issues))
        .route("/api/v1/roster", get(roster_get).put(roster_put))
        .route("/api/v1/history", get(history_list).post(history_add))
        .route("/api/v1/pty/spawn", post(pty_spawn))
        .route("/api/v1/pty/write", post(pty_write))
        .route("/api/v1/pty/resize", post(pty_resize))
        .route("/api/v1/pty/redraw", post(pty_redraw))
        .route("/api/v1/pty/kill", post(pty_kill))
        .route("/api/v1/pty/list", get(pty_list))
        .route("/api/v1/pty/{id}/tail", get(pty_tail))
        .route("/api/v1/pty/{id}/sleep-screen", get(pty_sleep_screen))
        .route("/api/v1/pty/submit", post(pty_submit))
        .route("/api/v1/pty/withdraw", post(pty_withdraw))
        .route("/api/v1/terminal/probe", post(terminal_probe))
        .route("/api/v1/terminal/probe-reply", post(terminal_probe_reply))
        .route("/api/v1/codex/allowed-folders", get(codex_allowed_folders))
        .route("/api/v1/codex/allow-folder", post(codex_allow_folder))
        .route("/api/v1/codex/withdraw-folder", post(codex_withdraw_folder))
        .merge(routes_hive::hive_router())
        .merge(routes_fs_git::fs_git_router())
        .merge(routes_silenced::silenced_router())
        .merge(routes_usage_memory_kg::usage_memory_kg_router())
        .merge(routes_integrations::integrations_router())
        .merge(routes_catalog::catalog_router())
        .merge(routes_agent_file::agent_file_router())
        .with_state(state)
        .layer(axum::middleware::from_fn(security::loopback_origin_guard))
        .layer(TraceLayer::new_for_http())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "md_server=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let data_dir = default_data_dir();
    let mut config = ConfigStore::new(&data_dir);
    let home = config.read().harness_home.map(PathBuf::from);
    let db = PersistStore::open(data_dir.join("harness.db"))?;
    let pty = PtyManager::new();
    let submitter = PtySubmitter::new(pty.clone());
    let control = Arc::new(Mutex::new(ControlRegistry::new()));
    let breaker = Arc::new(Mutex::new(CircuitBreaker::new()));
    let wake_filing = Arc::new(Mutex::new(WakeFiling::new()));
    let switch_clear = Arc::new(Mutex::new(SwitchClear::new()));
    let codex_size = Arc::new(Mutex::new(CodexSizeClear::new()));
    let hook_token = uuid::Uuid::new_v4().to_string();
    let hooks = HookEngine::new(control.clone(), breaker.clone(), hook_token);
    let (events_tx, _) = broadcast::channel(256);
    let workers = Arc::new(Mutex::new(Vec::new()));
    let ended_agents = Arc::new(Mutex::new(HashMap::new()));
    spawn_pty_event_forwarder(
        &pty,
        workers.clone(),
        wake_filing.clone(),
        switch_clear.clone(),
        codex_size.clone(),
        ended_agents.clone(),
        events_tx.clone(),
    );

    let secrets = md_integrations::secrets::SecretStore::open(&data_dir)
        .map_err(|e| anyhow::anyhow!(e))?;
    let webhooks = md_integrations::webhooks::WebhookRegistry::open(Some(&data_dir));

    let workers_history_path = data_dir.join("workers-history.json");
    let workers_history: Vec<serde_json::Value> = std::fs::read_to_string(&workers_history_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let port: u16 = std::env::var("MD_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3110);

    let state = AppState {
        port,
        data_dir: data_dir.clone(),
        config: Arc::new(Mutex::new(config)),
        db: Arc::new(Mutex::new(db)),
        roster: Arc::new(Mutex::new(RosterStore::new())),
        hive: Arc::new(Mutex::new(HiveManager::new(home))),
        control,
        breaker,
        wake_filing,
        switch_clear,
        codex_size,
        hooks,
        pty,
        submitter,
        probes: TerminalProbes::new(),
        ide: Arc::new(Mutex::new(IdeState {
            unsaved_count: 0,
            open_files: Vec::new(),
        })),
        secrets: Arc::new(Mutex::new(secrets)),
        broker: Arc::new(Mutex::new(md_integrations::secrets::IntegrationBroker::open(&data_dir))),
        webhooks: Arc::new(Mutex::new(webhooks)),
        file_share: Arc::new(Mutex::new(md_integrations::file_share::FileShareRegistry::new())),
        workers,
        workers_history: Arc::new(Mutex::new(workers_history)),
        ended_agents,
        integrations_activity: Arc::new(Mutex::new(Vec::new())),
        events_tx,
    };
    {
        let init_cfg = state.config.lock().unwrap().read();
        state.sync_config_runtime(&init_cfg);
    }
    let boot_roster = state
        .harness_home()
        .and_then(|home_dir| state.roster.lock().unwrap().read(Some(&home_dir)));
    if let Some(snap) = boot_roster {
        state.sync_hive_home();
        let _ = state
            .hive
            .lock()
            .unwrap()
            .sync_roster(&snap.agents, &snap.archived);
        crate::hive_bootstrap::sync_fleet_json(&state);
    }
    routes_hive::spawn_hive_poller(state.clone());

    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_state(dir: &std::path::Path) -> AppState {
        let pty = PtyManager::new();
        let submitter = PtySubmitter::with_timings(
            pty.clone(),
            md_pty::submitter::SubmitTimings::fast_for_test(),
        );
        let control = Arc::new(Mutex::new(ControlRegistry::new()));
        let breaker = Arc::new(Mutex::new(CircuitBreaker::new()));
        let wake_filing = Arc::new(Mutex::new(WakeFiling::new()));
        let switch_clear = Arc::new(Mutex::new(SwitchClear::new()));
        let codex_size = Arc::new(Mutex::new(CodexSizeClear::new()));
        let hooks = HookEngine::new(control.clone(), breaker.clone(), "test-hook-token".into());
        let (events_tx, _) = broadcast::channel(32);
        AppState {
            port: 3110,
            data_dir: dir.to_path_buf(),
            config: Arc::new(Mutex::new(ConfigStore::new(dir))),
            db: Arc::new(Mutex::new(PersistStore::open_in_memory().unwrap())),
            roster: Arc::new(Mutex::new(RosterStore::new())),
            hive: Arc::new(Mutex::new(HiveManager::new(None))),
            control,
            breaker,
            wake_filing,
            switch_clear,
            codex_size,
            hooks,
            pty,
            submitter,
            probes: TerminalProbes::new(),
            ide: Arc::new(Mutex::new(IdeState {
                unsaved_count: 0,
                open_files: Vec::new(),
            })),
            secrets: Arc::new(Mutex::new(md_integrations::secrets::SecretStore::open_in_memory())),
            broker: Arc::new(Mutex::new(md_integrations::secrets::IntegrationBroker::new())),
            webhooks: Arc::new(Mutex::new(md_integrations::webhooks::WebhookRegistry::open(None))),
            file_share: Arc::new(Mutex::new(md_integrations::file_share::FileShareRegistry::new())),
            workers: Arc::new(Mutex::new(Vec::new())),
            workers_history: Arc::new(Mutex::new(Vec::new())),
            ended_agents: Arc::new(Mutex::new(HashMap::new())),
            integrations_activity: Arc::new(Mutex::new(Vec::new())),
            events_tx,
        }
    }

    #[tokio::test]
    async fn health_config_roster_history_pty_and_hooks_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home_dir = dir.join("hive-home");
        let app = build_router(test_state(&dir));

        let res = app
            .clone()
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let patch_body = serde_json::json!({
            "harnessHome": home_dir.to_string_lossy(),
            "onboardingComplete": true
        });
        let cfg_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/config")
                    .header("content-type", "application/json")
                    .body(Body::from(patch_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cfg_res.status(), StatusCode::OK);

        // Unauthorized hook is rejected; authorized hook succeeds.
        let unauth_hook = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/michael")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"hook_event_name":"PreToolUse","tool_name":"Bash"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauth_hook.status(), StatusCode::UNAUTHORIZED);

        let auth_hook = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/michael")
                    .header("authorization", "Bearer test-hook-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"hook_event_name":"Status","context_window":{"total_input_tokens":1200,"context_window_size":200000}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(auth_hook.status(), StatusCode::OK);

        // Task creation through hive endpoint
        let task_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/tasks")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"MD-10","title":"Verify Phase 3"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(task_res.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn fs_git_and_ide_routes_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-fs-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let app = build_router(test_state(&dir));

        let create_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/fs/create")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "root": dir.to_string_lossy(),
                            "rel": "hello.txt"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_res.status(), StatusCode::OK);

        let write_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/fs/write")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "root": dir.to_string_lossy(),
                            "rel": "hello.txt",
                            "content": "phase-4-ok"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(write_res.status(), StatusCode::OK);

        let read_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/fs/read")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "root": dir.to_string_lossy(),
                            "rel": "hello.txt"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(read_res.status(), StatusCode::OK);

        let git_uri = format!("/api/v1/git/is-repo?cwd={}", dir.to_string_lossy());
        let git_res = app
            .clone()
            .oneshot(Request::builder().uri(git_uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(git_res.status(), StatusCode::OK);

        let ide_dirty_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ide/dirty")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"unsaved":2,"openFiles":["hello.txt"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ide_dirty_res.status(), StatusCode::OK);

        let ide_page_res = app
            .clone()
            .oneshot(Request::builder().uri("/ide").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(ide_page_res.status(), StatusCode::OK);

        let ide_state_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ide/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ide_state_res.status(), StatusCode::OK);

        for uri in [
            "/api/v1/cloud/list",
            "/api/v1/connections/list",
            "/api/v1/update/status",
            "/api/v1/license/status",
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "failed silenced GET {uri}");
        }

        let analytics_res = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/analytics/funnel")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(analytics_res.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn usage_memory_and_kg_routes_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-phase5-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = dir.join("hive-home");
        std::fs::create_dir_all(&home).unwrap();
        let app = build_router(test_state(&dir));

        let cfg_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "harnessHome": home.to_string_lossy() }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cfg_res.status(), StatusCode::OK);

        let limit_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/config/token-limit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentId":"michael","limit":500000}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(limit_res.status(), StatusCode::OK);

        let mem_put = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/memory/michael")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"content":"Always verify with cargo test."}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mem_put.status(), StatusCode::OK);

        let mem_search = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/memory/search?q=cargo")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(mem_search.status(), StatusCode::OK);

        let kg_ingest = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/kg/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"title":"Guide","text":"Floor sizing preserves viewport aspect."}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(kg_ingest.status(), StatusCode::OK);

        let kg_search = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/kg/search?q=viewport")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(kg_search.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn integrations_webhooks_share_and_realtime_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-phase6-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("report.txt"), "hello-share").unwrap();
        let app = build_router(test_state(&dir));

        let set_secret = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/secrets")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"keyRef":"apikey:openai","kind":"provider","value":"sk-secret-12345"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(set_secret.status(), StatusCode::OK);

        let rt_key = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/realtime/has-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rt_key.status(), StatusCode::OK);

        let wh_upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/webhooks/endpoints")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"id":"gh","name":"GitHub","secret":"wh-sec","mode":"awaiting-approval"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wh_upsert.status(), StatusCode::OK);

        let wh_post = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/gh")
                    .header("content-type", "application/json")
                    .header("x-md-webhook-secret", "wh-sec")
                    .body(Body::from(r#"{"message":"build passed"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wh_post.status(), StatusCode::ACCEPTED);

        let share_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/share")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "root": dir.to_string_lossy(),
                            "rel": "report.txt"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(share_res.status(), StatusCode::OK);

        let save_int = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/integrations/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "record": {
                                "id": "stripe",
                                "label": "Stripe",
                                "kind": "custom-rest",
                                "baseUrl": "https://api.stripe.com/v1",
                                "authType": "bearer",
                                "secretRef": "int:stripe",
                                "templateId": "stripe",
                                "enabled": true,
                                "createdAt": 1,
                                "updatedAt": 1
                            },
                            "secret": "sk_test_123456"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(save_int.status(), StatusCode::OK);

        let list_int = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/integrations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_int.status(), StatusCode::OK);
        let list_bytes = axum::body::to_bytes(list_int.into_body(), usize::MAX).await.unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&list_bytes).unwrap();
        assert_eq!(list_json[0]["id"], "stripe");
        assert_eq!(list_json[0]["hasSecret"], true);

        let del_int = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/integrations/stripe")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_int.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn frontend_and_catalog_routes_work() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-phase7-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let app = build_router(test_state(&dir));

        for uri in [
            "/",
            "/cth-web.js",
            "/ide.html",
            "/static/styles.css",
            "/static/app.js",
            "/api/v1/templates",
            "/api/v1/presets",
            "/api/v1/skills",
            "/api/v1/themes/workbench",
        ] {
            let res = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK, "failed GET {uri}");
        }

        let export_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agents/export")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"michael","name":"Michael"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(export_res.status(), StatusCode::OK);

        // Cross-origin & DNS-rebinding requests are rejected with 403.
        let evil_origin = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/tasks")
                    .header("origin", "https://evil.example")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"id":"MD-99","title":"Blocked"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(evil_origin.status(), StatusCode::FORBIDDEN);

        let evil_host = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/config")
                    .header("host", "evil.example:3110")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(evil_host.status(), StatusCode::FORBIDDEN);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn fixes_regression_suite_works() {
        let dir = std::env::temp_dir().join(format!(
            "md-srv-fixes-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let home = dir.join("hive-home");
        std::fs::create_dir_all(&home).unwrap();
        let state = test_state(&dir);
        let app = build_router(state.clone());

        // 1. PtyEvent::Exit serializes camelCase fields (exitCode, startedAt).
        let exit_json = serde_json::to_value(PtyEvent::Exit {
            id: "pty-god".to_string(),
            exit_code: 0,
            signal: None,
            tail: String::new(),
            started_at: 42,
        })
        .unwrap();
        assert_eq!(exit_json["exitCode"], 0);
        assert_eq!(exit_json["startedAt"], 42);
        assert!(exit_json.get("exit_code").is_none());

        // 2. memory_status honors semanticMemory & embeddingModel config keys.
        let cfg_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "harnessHome": home.to_string_lossy(),
                            "semanticMemory": false,
                            "embeddingModel": "embeddinggemma"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(cfg_res.status(), StatusCode::OK);
        let mem_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/memory/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let mem_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(mem_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(mem_json["enabled"], false);
        assert_eq!(mem_json["model"], "embeddinggemma");

        // 3. Token limit accepts { period, cap } objects and token-limits/view returns structured view.
        let lim_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/config/token-limit")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentId":"god","limit":{"period":"week","cap":250000}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(lim_res.status(), StatusCode::OK);
        let view_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/token-limits/view")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentIds":["god"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let view_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(view_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(view_json["agents"]["god"]["limit"]["cap"], 250000);
        assert_eq!(view_json["agents"]["god"]["period"], "week");

        // 4. Webhooks batch save & status work, and persist disabled flag.
        let wh_save = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/webhooks/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"[{"id":"wh-1","name":"CI","secret":"sec-1","mode":"auto-dispatch","enabled":true}]"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wh_save.status(), StatusCode::OK);
        let wh_st = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/webhooks/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let wh_st_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(wh_st.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(wh_st_json["running"], true);
        assert_eq!(wh_st_json["endpoints"][0]["id"], "wh-1");

        // 5. Integration delete removes custom secretRef cleanly.
        let save_int = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/integrations/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"record":{"id":"gh","label":"GH","baseUrl":"https://api.github.com","authType":"bearer","secretRef":"custom:gh-ref"},"secret":"ghp_secret123"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(save_int.status(), StatusCode::OK);
        let del_int = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/integrations/gh")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(del_int.status(), StatusCode::OK);
        assert!(!state.secrets.lock().unwrap().has_secret("custom:gh-ref"));

        // 6. ide/dirty clears openFiles when empty array is sent.
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ide/dirty")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"unsaved":1,"openFiles":["a.rs"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ide/dirty")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"unsaved":0,"openFiles":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(state.ide.lock().unwrap().open_files.is_empty());

        // 7. hive/send includes targets array and hive/human-mail-since surfaces human mail.
        let send_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/send")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"from":"god","msg":{"to":"human","subject":"Check","body":"Ready for review"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let send_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(send_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(send_json["message"]["targets"][0], "human");

        let mail_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/human-mail-since")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let mail_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(mail_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(mail_json["god"]["count"], 1);

        // 8. tools/status includes opencode.
        let tools_res = app
            .oneshot(Request::builder().uri("/api/v1/tools/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let tools_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(tools_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(
            tools_json
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == "engine:opencode")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn agent_file_templates_and_extended_endpoints_work() {
        let dir = std::env::temp_dir().join(format!("md-server-ext-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = test_state(&dir);
        state.config.lock().unwrap().update(serde_json::json!({
            "harnessHome": dir.to_string_lossy().into_owned(),
            "registeredRepos": [dir.to_string_lossy().into_owned()]
        })).unwrap();
        let app = build_router(state.clone());

        let sample_file = serde_json::json!({
            "spec": "munder-difflin/agent@2",
            "name": "Research Analyst",
            "description": "Public source analyst",
            "parts": ["configs", "mcpServers", "skills", "memories"],
            "configs": {
                "provider": "claude",
                "role": "Research markets",
                "goal": "Write grounded briefs"
            },
            "mcpServers": [
                { "id": "mcp-fetch", "command": "npx", "args": ["-y", "fetch-mcp"] }
            ],
            "skills": [
                { "name": "brief-writer", "description": "Brief skill", "files": { "SKILL.md": "# Brief Writer\n" } }
            ],
            "memories": [
                { "content": "Always cite primary sources." }
            ]
        });

        // 1. agentFilePlan
        let plan_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent-file/plan")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "agentId": null, "file": sample_file }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(plan_res.status(), StatusCode::OK);
        let plan_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(plan_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(plan_json["ok"], true);
        assert!(!plan_json["changes"].as_array().unwrap().is_empty());

        // Seed analyst-1 in roster first (as UI does before agentFileApply / agentFileExport)
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/roster")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "agents": [{
                                "id": "analyst-1",
                                "name": "Research Analyst",
                                "provider": "claude",
                                "description": "Initial role"
                            }],
                            "archived": [],
                            "restorable": []
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        // 2. agentFileApply -> agentFileExport -> skill-choices
        let accept_ids: Vec<String> = plan_json["changes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| c["id"].as_str().map(ToString::to_string))
            .collect();
        let apply_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent-file/apply")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "agentId": "analyst-1",
                            "file": sample_file,
                            "accept": accept_ids,
                            "fresh": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let apply_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(apply_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(apply_json["ok"], true);
        assert_eq!(apply_json["card"]["name"], "Research Analyst");

        let exp_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent-file/export")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "agentId": "analyst-1", "parts": ["configs", "memories", "skills"] }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let exp_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(exp_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(exp_json["ok"], true);
        assert!(exp_json["json"].as_str().unwrap().contains("Research Analyst"));

        // 3. template save + list + delete
        let save_tpl = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/templates/pro/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "name": "My Custom Template",
                            "description": "Saved from web",
                            "file": sample_file,
                            "accepted": ["configs"]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let save_tpl_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(save_tpl.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(save_tpl_json["ok"], true);
        let tpl_id = save_tpl_json["id"].as_str().unwrap().to_string();

        let del_tpl = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/templates/pro/{tpl_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let del_tpl_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(del_tpl.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(del_tpl_json["ok"], true);

        // 4. capabilitiesInstalled includes tools; agents/settings updates config
        let cap_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/catalog/capabilities/installed")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"cwds":[],"agentIds":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let cap_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(cap_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(cap_json["tools"].is_array());

        let set_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agents/settings")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentId":"analyst-1","kind":"codexMemory","value":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let set_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(set_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(set_json["agentCodexMemoryOn"].as_array().unwrap().iter().any(|x| x == "analyst-1"));

        // 5. fs/upload + fs/raw (and 403 outside allowed roots) + share/inspect
        let up_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/fs/upload")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"name":"note.txt","base64":"SGVsbG8gd29ybGQ="}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let up_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(up_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(up_json["ok"], true);
        let uploaded_path = up_json["path"].as_str().unwrap().to_string();

        let raw_ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/fs/raw?path={uploaded_path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(raw_ok.status(), StatusCode::OK);
        let raw_bytes = axum::body::to_bytes(raw_ok.into_body(), usize::MAX).await.unwrap();
        assert_eq!(raw_bytes.as_ref(), b"Hello world");

        let raw_forbidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/fs/raw?path=/etc/hosts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(raw_forbidden.status(), StatusCode::FORBIDDEN);

        let inspect_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/share/inspect")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::json!({ "paths": [uploaded_path] }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let inspect_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(inspect_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(inspect_json[0]["ok"], true);
        assert_eq!(inspect_json[0]["size"], 11);

        // 6. kg/upload + integrations/activity + wake + screen-lines + usage/limits
        let kg_up = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/kg/upload")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"fileName":"brief.md","text":"Quantum caching design notes."}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let kg_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(kg_up.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(kg_json["ok"], true);

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/integrations/save")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"record":{"id":"crm","label":"CRM","baseUrl":"https://example.com","authType":"bearer"},"secret":"tok"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let act_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/integrations/activity").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let act_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(act_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(!act_json.as_array().unwrap().is_empty());

        let limits_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/usage/limits").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let limits_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(limits_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(limits_json["engines"].as_array().unwrap().len(), 3);
        assert!(limits_json["engines"][0]["windows"].is_array());
        assert_eq!(limits_json["engines"][0]["engine"], "claude");

        let lines_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/pty/screen-lines")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"ptyId":"none","lines":10}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let lines_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(lines_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(lines_json["ok"], true);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn hive_bootstrap_hooks_temp_comms_and_wake_work() {
        let dir = std::env::temp_dir().join(format!("md-hive-e2e-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = test_state(&dir);
        state
            .config
            .lock()
            .unwrap()
            .update(serde_json::json!({
                "harnessHome": dir.to_string_lossy().into_owned(),
                "onboardingComplete": true
            }))
            .unwrap();
        let app = build_router(state.clone());
        let mut rx = state.events_tx.subscribe();

        // 1. Spawning Michael bootstraps COMMANDS.md, fleet.json, agent dirs, memory.md, and returns seedPrompt
        let spawn_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/pty/spawn")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "id": "pty-god",
                            "cwd": dir.to_string_lossy().into_owned(),
                            "command": "/bin/sh",
                            "args": ["-c", "echo ready; cat >/dev/null"],
                            "provider": "opencode",
                            "resume": true,
                            "hive": { "id": "god", "name": "Michael", "provider": "opencode", "isGod": true }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let spawn_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(spawn_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(spawn_json["ok"], true);
        assert_eq!(spawn_json["resumed"], false);
        let seed = spawn_json["seedPrompt"].as_str().unwrap();
        assert!(seed.contains("COMMANDS.md"));
        assert!(seed.contains("spawn-requests"));

        let hive_root = dir.join("hive");
        assert!(hive_root.join("COMMANDS.md").is_file());
        assert!(hive_root.join("fleet.json").is_file());
        assert!(hive_root.join("agents/god/inbox").is_dir());
        assert!(hive_root.join("agents/god/outbox").is_dir());
        assert!(hive_root.join("agents/god/memory.md").is_file());

        // 2. hive/send emits hive:enqueue for non-human recipients & Stop hook blocks while inbox non-empty
        let send_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/send")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"from":"human","msg":{"to":"god","act":"propose","subject":"Board update","body":"Add Q4 plan"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let send_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(send_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(send_json["ok"], true);

        let mut saw_enqueue = false;
        let mut saw_from_inbox = false;
        while let Ok(ev) = rx.try_recv() {
            if ev.channel == "hive:enqueue" && ev.payload["targetId"] == "god" {
                saw_enqueue = true;
                saw_from_inbox = ev.payload["fromInbox"] == true;
            }
        }
        assert!(saw_enqueue, "hive_send should emit hive:enqueue to wake god");
        assert!(saw_from_inbox, "hive_send enqueue should carry fromInbox: true");

        let hook_stop = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/god")
                    .header("authorization", "Bearer test-hook-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"hook_event_name":"Stop"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let hook_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(hook_stop.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(
            hook_json.get("decision").is_none(),
            "opencode Stop hook should never return blocking decision (avoids UI idle deadlock): {hook_json}"
        );

        // 2b. wake-text returns structured wake -> pty_submit + Stop auto-files inbox to inbox/.done
        let wake_text_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hive/wake-text/god")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let wake_text_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(wake_text_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        let wake_str = wake_text_json.as_str().expect("wake-text should return string");
        assert!(wake_str.contains("You have new hive inbox message(s): 1, printed below."));
        assert!(wake_str.contains("These messages are filed for you when your turn ends:"));

        let submit_stuck = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/pty/submit")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "ptyId": "pty-god",
                            "agentId": "god",
                            "provider": "opencode",
                            "text": wake_str,
                            "source": "queue"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let stuck_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(submit_stuck.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(stuck_json["ok"], true);
        assert_eq!(stuck_json["sent"], false);
        assert_eq!(stuck_json["result"]["outcome"], "stuck");

        // Now deliver with a simulated UserPromptSubmit hook triggered on `pty:began`
        let app_hook = app.clone();
        let wake_for_hook = wake_str.to_string();
        let mut began_rx = state.events_tx.subscribe();
        tokio::spawn(async move {
            while let Ok(ev) = began_rx.recv().await {
                if ev.channel == "pty:began" {
                    let _ = app_hook
                        .oneshot(
                            Request::builder()
                                .method("POST")
                                .uri("/hook/god")
                                .header("authorization", "Bearer test-hook-token")
                                .header("content-type", "application/json")
                                .body(Body::from(
                                    serde_json::json!({
                                        "hook_event_name": "UserPromptSubmit",
                                        "prompt": wake_for_hook
                                    })
                                    .to_string(),
                                ))
                                .unwrap(),
                        )
                        .await;
                    break;
                }
            }
        });

        let submit_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/pty/submit")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "ptyId": "pty-god",
                            "agentId": "god",
                            "provider": "opencode",
                            "text": wake_str,
                            "source": "queue"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let submit_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(submit_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(submit_json["ok"], true);
        assert_eq!(submit_json["sent"], true);
        assert_eq!(submit_json["result"]["outcome"], "submitted");
        assert_eq!(submit_json["result"]["proof"], "hook");

        let hook_stop_after = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/god")
                    .header("authorization", "Bearer test-hook-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"hook_event_name":"Stop"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let hook_after_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(hook_stop_after.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(
            hook_after_json.get("decision").is_none(),
            "Stop should not block after WakeFiling files the inbox message: {hook_after_json}"
        );
        assert!(state.hive.lock().unwrap().inbox("god").is_empty());
        assert!(hive_root.join("agents/god/inbox/.done").is_dir());

        // 3. Temp spawn request spawns worker, routes outbox back to boss inbox, and handles exit pruning
        let req_dir = hive_root.join("spawn-requests");
        std::fs::create_dir_all(&req_dir).unwrap();
        std::fs::write(
            req_dir.join("test-worker.json"),
            serde_json::json!({
                "name": "Temp Analyst",
                "objective": "Write report and reply",
                "cwd": dir.to_string_lossy().into_owned(),
                "command": "/bin/sh -c true",
                "provider": "opencode"
            })
            .to_string(),
        )
        .unwrap();
        routes_hive::process_spawn_requests(&state);
        assert!(hive_root.join("agents/worker-test-worker/outbox").is_dir());

        // Write a message from temp's outbox -> drain_outboxes -> arrives in god's inbox
        let out_file = hive_root.join("agents/worker-test-worker/outbox/done-1.json");
        std::fs::write(
            &out_file,
            r#"{"to":"god","act":"done","subject":"Finished","body":"Report saved"}"#,
        )
        .unwrap();
        let routed = state.hive.lock().unwrap().drain_outboxes();
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0]["from"], "worker-test-worker");
        assert!(state
            .hive
            .lock()
            .unwrap()
            .inbox("god")
            .iter()
            .any(|m| m["from"] == "worker-test-worker" && m["act"] == "done"));

        // Wait briefly for `/bin/sh -c exit 0` to exit then prune
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        routes_hive::prune_exited_workers(&state);
        let hist_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/workers/history").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let hist_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(hist_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert!(hist_json
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["workerId"] == "worker-test-worker"));

        let _ = state.pty.kill("pty-god");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn autonomy_delivery_breaker_tools_and_info_work() {
        let dir = std::env::temp_dir().join(format!("md-autonomy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let state = test_state(&dir);
        let app = build_router(state.clone());

        // 1. /api/v1/info exposes host platform and arch
        let info_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/info").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let info_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(info_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(info_json["platform"], md_core::host_node_platform());
        assert_eq!(info_json["arch"], md_core::host_node_arch());

        // 2. /api/v1/tools/status returns prerequisite, memory, and engine rows
        let tools_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/tools/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let tools_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(tools_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let tools_arr = tools_json.as_array().unwrap();
        assert!(tools_arr.iter().any(|t| t["id"] == "uv" && t["kind"] == "prerequisite"));
        assert!(tools_arr.iter().any(|t| t["id"] == "mempalace" && t["kind"] == "memory"));
        assert!(tools_arr.iter().any(|t| t["id"] == "engine:claude" && t["kind"] == "engine"));

        // 3. control/auto-delivery persists autoDeliveryPausedAgents and toggles delivery control
        let ad_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/control/auto-delivery")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentId":"god","on":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let ad_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(ad_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(ad_json["ok"], true);
        assert_eq!(ad_json["snapshot"]["autoDeliveryPaused"], true);
        assert_eq!(
            state.config.lock().unwrap().read().extra["autoDeliveryPausedAgents"]["god"],
            true
        );

        // 4. orchestratorMaySpawn=false keeps spawn-requests paused, maxConcurrentWorkers=2 reflects on /api/v1/workers
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "harnessHome": dir.to_string_lossy().into_owned(),
                            "orchestratorMaySpawn": false,
                            "maxConcurrentWorkers": 2,
                            "circuitBreaker": { "enabled": true, "hardStop": true, "repeatedToolLimit": 4 }
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(state.breaker.lock().unwrap().config().hard_stop);
        let workers_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/workers").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let workers_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(workers_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(workers_json["maxWorkers"], 2);

        let hive_root = dir.join("hive");
        std::fs::create_dir_all(hive_root.join("spawn-requests")).unwrap();
        std::fs::write(
            hive_root.join("spawn-requests/blocked-temp.json"),
            serde_json::json!({
                "objective": "Hold until enabled",
                "cwd": hive_root.join("projects/demo").to_string_lossy().into_owned(),
                "command": "/bin/sh -c true"
            })
            .to_string(),
        )
        .unwrap();
        routes_hive::process_spawn_requests(&state);
        assert!(hive_root.join("spawn-requests/blocked-temp.json").is_file());

        // Turn orchestratorMaySpawn back on -> spawns & creates hive/projects/demo
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri("/api/v1/config")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"orchestratorMaySpawn":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        routes_hive::process_spawn_requests(&state);
        assert!(hive_root.join("spawn-requests/.done/blocked-temp.json").is_file());
        assert!(hive_root.join("projects/demo").is_dir());
        assert!(hive_root.join("PROTOCOL.md").is_file());
        assert!(hive_root.join("memory-index.md").is_file());

        // 5. /api/v1/pty/{id}/sleep-screen returns "" for live PTY
        let _ = state.pty.spawn(md_pty::manager::SpawnOptions {
            id: "pty-live-test".to_string(),
            cwd: dir.to_string_lossy().into_owned(),
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "sleep 1".to_string()],
            cols: None,
            rows: None,
            env: std::collections::HashMap::new(),
            shell_script: None,
            provider: None,
            resume: None,
            hive: None,
        });
        let ss_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/pty/pty-live-test/sleep-screen")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ss_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(ss_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(ss_json["screen"], "");
        let _ = state.pty.kill("pty-live-test");

        // 6. workspaces/ensure creates dirs under harnessHome & opencode plugin/permissions get injected
        let ws_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/workspaces/ensure")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path":"Projects"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let ws_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(ws_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(ws_json["ok"], true);
        assert!(dir.join("Projects").is_dir());

        let mut oc_opts = md_pty::manager::SpawnOptions {
            id: "pty-god".to_string(),
            cwd: dir.join("Projects").to_string_lossy().into_owned(),
            command: "opencode".to_string(),
            args: vec!["--auto".to_string()],
            cols: None,
            rows: None,
            env: std::collections::HashMap::new(),
            shell_script: None,
            provider: Some("opencode".to_string()),
            resume: None,
            hive: None,
        };
        let _ = crate::hive_bootstrap::prepare_agent_spawn(
            &state,
            &mut oc_opts,
            "god",
            "Michael",
            "opencode",
            true,
            None,
        );
        let oc_cfg: serde_json::Value = serde_json::from_str(
            oc_opts
                .env
                .get("OPENCODE_CONFIG_CONTENT")
                .expect("OPENCODE_CONFIG_CONTENT should be set"),
        )
        .unwrap();
        assert_eq!(oc_cfg["permission"]["question"], "deny");
        assert_eq!(oc_cfg["permission"]["*"], "allow");
        assert!(hive_root.join("agents/god/opencode-hive-plugin.mjs").is_file());

        // 7. context-trigger defaults + webhook inbound auto-dispatch into god inbox
        let ct_res = app
            .clone()
            .oneshot(Request::builder().uri("/api/v1/context-trigger").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let ct_json: serde_json::Value =
            serde_json::from_slice(&axum::body::to_bytes(ct_res.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(ct_json["compact"]["enabled"], true);

        let _ = state
            .webhooks
            .lock()
            .unwrap()
            .upsert_endpoint(md_integrations::webhooks::WebhookEndpointInput {
                id: "ci-hook".to_string(),
                name: "CI".to_string(),
                secret: "topsecret".to_string(),
                mode: md_integrations::webhooks::TriggerMode::AutoDispatch,
                extra: Default::default(),
            });
        let wh_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhooks/ci-hook")
                    .header("content-type", "application/json")
                    .header("x-md-webhook-secret", "topsecret")
                    .body(Body::from(r#"{"message":"Deploy completed","title":"Build #42","from":"CI"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wh_res.status(), StatusCode::OK);
        assert!(state
            .hive
            .lock()
            .unwrap()
            .inbox("god")
            .iter()
            .any(|m| m["from"] == "webhook" && m["body"].as_str().unwrap_or("").contains("Deploy completed")));

        // 8. Calling /api/v1/hive/wake on a live PTY while WS listener is subscribed must NOT emit `hive:enqueue` loop
        let mut wake_rx = state.events_tx.subscribe();
        let _ = state.pty.spawn(md_pty::manager::SpawnOptions {
            id: "pty-god".to_string(),
            cwd: dir.to_string_lossy().into_owned(),
            command: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "sleep 1".to_string()],
            cols: None,
            rows: None,
            env: std::collections::HashMap::new(),
            shell_script: None,
            provider: None,
            resume: None,
            hive: None,
        });
        let wake_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hive/wake")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"agentId":"god"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wake_res.status(), StatusCode::OK);
        while let Ok(ev) = wake_rx.try_recv() {
            assert_ne!(
                ev.channel, "hive:enqueue",
                "hive_wake must never re-emit hive:enqueue back into the UI queue on live PTYs"
            );
        }
        let _ = state.pty.kill("pty-god");

        // 9. /api/v1/hive/ended-agents returns exited non-worker agents and omits live ones
        state.ended_agents.lock().unwrap().insert(
            "analyst-1".to_string(),
            state::EndedAgentRecord {
                agent_id: "analyst-1".to_string(),
                exit_code: 0,
                signal: None,
                print_mode: false,
                at: 12345,
            },
        );
        let ended_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hive/ended-agents")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ended_json: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(ended_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(ended_json
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["agentId"] == "analyst-1"));

        // 10. switch-clear-gate holds when a subagent tool call is open and releases on PostToolUse
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/god")
                    .header("authorization", "Bearer test-hook-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"hook_event_name":"PreToolUse","tool_name":"Agent","tool_use_id":"sub-1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let gate_hold = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hive/switch-clear-gate/god")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let gate_hold_val: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(gate_hold.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(gate_hold_val, "hold");

        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/hook/god")
                    .header("authorization", "Bearer test-hook-token")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"hook_event_name":"PostToolUse","tool_name":"Agent","tool_use_id":"sub-1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let gate_go = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hive/switch-clear-gate/god")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let gate_go_val: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(gate_go.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(gate_go_val, "go");

        // 11. codex-size-clear-text returns null for non-codex or under-threshold codex agents
        let size_res = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hive/codex-size-clear-text/god")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let size_val: serde_json::Value = serde_json::from_slice(
            &axum::body::to_bytes(size_res.into_body(), usize::MAX)
                .await
                .unwrap(),
        )
        .unwrap();
        assert!(size_val.is_null());

        std::fs::remove_dir_all(&dir).ok();
    }
}
