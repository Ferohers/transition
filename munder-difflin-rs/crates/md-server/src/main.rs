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
};
use md_pty::{
    manager::{PtyEvent, PtyListEntry, PtyManager, SpawnOptions, SpawnResult},
    submitter::{PtySubmitRequest, PtySubmitter, TerminalProbeReply, TerminalProbes},
};
use serde::Deserialize;
use state::{AppState, IdeState, ServerEvent};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tower_http::services::ServeDir;

fn spawn_pty_event_forwarder(pty: &PtyManager, events_tx: broadcast::Sender<ServerEvent>) {
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
                        payload: serde_json::json!({ "id": id, "exitCode": exit_code, "signal": signal }),
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
    Json(opts): Json<SpawnOptions>,
) -> Json<SpawnResult> {
    Json(state.pty.spawn(opts))
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

async fn pty_submit(
    State(state): State<AppState>,
    Json(req): Json<PtySubmitRequest>,
) -> Json<serde_json::Value> {
    let state_clone = state.clone();
    match state
        .submitter
        .submit(req, move |r| {
            state_clone.emit("pty:began", serde_json::json!({ "ptyId": r.pty_id }));
        })
        .await
    {
        Ok(res) => serde_json::to_value(res).map(Json).unwrap_or_else(|_| {
            Json(serde_json::json!({ "ok": false, "error": "serialization error" }))
        }),
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
    for candidate in [
        "/Users/altan/Documents/transition/munder-difflin-rs/web",
        "./web",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_dir() {
            return p;
        }
    }
    PathBuf::from("./web")
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
    let hook_token = uuid::Uuid::new_v4().to_string();
    let hooks = HookEngine::new(control.clone(), breaker.clone(), hook_token);
    let (events_tx, _) = broadcast::channel(256);
    spawn_pty_event_forwarder(&pty, events_tx.clone());

    let secrets = md_integrations::secrets::SecretStore::open(&data_dir)
        .map_err(|e| anyhow::anyhow!(e))?;
    let webhooks = md_integrations::webhooks::WebhookRegistry::open(Some(&data_dir));

    let state = AppState {
        config: Arc::new(Mutex::new(config)),
        db: Arc::new(Mutex::new(db)),
        roster: Arc::new(Mutex::new(RosterStore::new())),
        hive: Arc::new(Mutex::new(HiveManager::new(home))),
        control,
        breaker,
        hooks,
        pty,
        submitter,
        probes: TerminalProbes::new(),
        ide: Arc::new(Mutex::new(IdeState {
            unsaved_count: 0,
            open_files: Vec::new(),
        })),
        secrets: Arc::new(Mutex::new(secrets)),
        broker: Arc::new(Mutex::new(md_integrations::secrets::IntegrationBroker::new())),
        webhooks: Arc::new(Mutex::new(webhooks)),
        file_share: Arc::new(Mutex::new(md_integrations::file_share::FileShareRegistry::new())),
        events_tx,
    };

    let port: u16 = std::env::var("MD_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3110);
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
        let submitter = PtySubmitter::new(pty.clone());
        let control = Arc::new(Mutex::new(ControlRegistry::new()));
        let breaker = Arc::new(Mutex::new(CircuitBreaker::new()));
        let hooks = HookEngine::new(control.clone(), breaker.clone(), "test-hook-token".into());
        let (events_tx, _) = broadcast::channel(32);
        AppState {
            config: Arc::new(Mutex::new(ConfigStore::new(dir))),
            db: Arc::new(Mutex::new(PersistStore::open_in_memory().unwrap())),
            roster: Arc::new(Mutex::new(RosterStore::new())),
            hive: Arc::new(Mutex::new(HiveManager::new(None))),
            control,
            breaker,
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
}
