mod routes_fs_git;
mod routes_hive;
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

async fn index_html() -> Html<&'static str> {
    Html(
        r#"<!doctype html>
<html lang="en" data-cth-theme="dark" data-cth-skin="professional">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Munder Difflin (Rust Web)</title>
  <style>
    :root {
      --cth-cream-50: #17171B;
      --cth-paper-100: #1A1A1F;
      --cth-ink-900: #DEDBD6;
      --cth-ink-500: #96919F;
      --cth-ink-300: #787684;
      --cth-accent: #CFAA57;
    }
    * { box-sizing: border-box; }
    html, body { margin: 0; height: 100%; background: var(--cth-cream-50); color: var(--cth-ink-900); font-family: Inter, system-ui, sans-serif; font-size: 13px; }
    .shell { display: flex; height: 100dvh; }
    .rail { width: 240px; border-right: 1px solid var(--cth-ink-300); background: var(--cth-paper-100); padding: 12px; display: flex; flex-direction: column; gap: 6px; }
    .rail h1 { font-size: 12.5px; margin: 0; letter-spacing: 0.04em; text-transform: uppercase; }
    .meta { font-size: 10.5px; color: var(--cth-ink-500); }
    .stage { flex: 1; display: grid; place-items: center; padding: 0; overflow: hidden; container-type: size; }
    .floor { border: 1px solid var(--cth-ink-300); background: #222229; display: grid; place-items: center; font-size: 12px; color: var(--cth-ink-500); content-visibility: auto; contain-intrinsic-size: auto 512px auto 324px; }
  </style>
</head>
<body>
  <div class="shell">
    <aside class="rail">
      <h1>Munder Difflin RS</h1>
      <div class="meta">Pro View Compact Density (10.5–13px)</div>
      <div class="meta" id="cfg-status">Loading config…</div>
      <div class="meta" id="ws-status">WS: connecting…</div>
    </aside>
    <main class="stage" id="stage">
      <div class="floor" id="floor">Enlarged Office Stage (homeBoxOf)</div>
    </main>
  </div>
  <script>
    const stage = document.getElementById('stage');
    const floor = document.getElementById('floor');
    const wsStatus = document.getElementById('ws-status');

    async function layoutFloor() {
      if (document.hidden) return;
      const res = await fetch(`/api/v1/floor/box?home_w=${stage.clientWidth}&stage_h=${stage.clientHeight}`);
      const box = await res.json();
      floor.style.width = `${box.w}px`;
      floor.style.height = `${box.h}px`;
      floor.textContent = `Office Stage: ${box.w} × ${box.h}px (aspect-fit viewport)`;
    }

    async function boot() {
      const [_, cfgRes] = await Promise.all([layoutFloor(), fetch('/api/v1/config')]);
      const cfg = await cfgRes.json();
      document.getElementById('cfg-status').textContent = `Skin: ${cfg.skin} · Theme: ${cfg.theme}`;
    }

    function connectWs() {
      const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      const ws = new WebSocket(`${proto}//${location.host}/ws/events`);
      ws.onopen = () => { wsStatus.textContent = 'WS: live'; };
      ws.onclose = () => { wsStatus.textContent = 'WS: reconnecting…'; setTimeout(connectWs, 1500); };
      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data);
          if (msg.channel === 'config:changed') {
            document.getElementById('cfg-status').textContent = `Skin: ${msg.payload.skin} · Theme: ${msg.payload.theme}`;
          }
        } catch {}
      };
    }

    let floorSkipped = false;
    floor.addEventListener('contentvisibilityautostatechange', (ev) => {
      floorSkipped = ev.skipped;
      if (!ev.skipped) void layoutFloor();
    });
    new ResizeObserver(() => { if (!floorSkipped) void layoutFloor(); }).observe(stage);
    document.addEventListener('visibilitychange', () => { if (!document.hidden && !floorSkipped) void layoutFloor(); });
    void boot();
    connectWs();
  </script>
</body>
</html>"#,
    )
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index_html))
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
        .with_state(state)
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
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ide/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ide_state_res.status(), StatusCode::OK);

        std::fs::remove_dir_all(&dir).ok();
    }
}
