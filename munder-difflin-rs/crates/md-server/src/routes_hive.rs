use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use md_hive::breaker::BreakerState;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentToggleReq {
    agent_id: String,
    on: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GateToolReq {
    agent_id: String,
    tool: String,
    on: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SteerReq {
    agent_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentIdReq {
    agent_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PatchFieldReq {
    id: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogQuery {
    n: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessagesQuery {
    agent_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: Option<String>,
}

async fn handle_hook(
    Path(agent_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    let auth = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if !state.hooks.token_valid(auth) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let (reply, side) = state
        .hooks
        .handle_hook(&agent_id, &payload, None, None, None, None);
    for (ch, ev) in side.events {
        state.emit(&ch, ev);
    }
    Ok(Json(reply))
}

async fn control_pause(State(state): State<AppState>, Json(req): Json<AgentToggleReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.pause(&req.agent_id, req.on);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_auto_delivery(
    State(state): State<AppState>,
    Json(req): Json<AgentToggleReq>,
) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.pause_auto_delivery(&req.agent_id, req.on);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_resume(State(state): State<AppState>, Json(req): Json<AgentIdReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.resume(&req.agent_id);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_gate_tool(State(state): State<AppState>, Json(req): Json<GateToolReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.gate_tool(&req.agent_id, &req.tool, req.on);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_steer(State(state): State<AppState>, Json(req): Json<SteerReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.steer(&req.agent_id, &req.text);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_halt(State(state): State<AppState>, Json(req): Json<AgentIdReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.halt(&req.agent_id);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_unhalt(State(state): State<AppState>, Json(req): Json<AgentIdReq>) -> Json<Value> {
    let snap = {
        let mut ctl = state.control.lock().unwrap();
        ctl.unhalt(&req.agent_id);
        ctl.snapshot(&req.agent_id)
    };
    Json(serde_json::json!({ "ok": true, "snapshot": snap }))
}

async fn control_snapshot(Path(id): Path<String>, State(state): State<AppState>) -> Json<Value> {
    let snap = state.control.lock().unwrap().snapshot(&id);
    Json(serde_json::to_value(snap).unwrap_or(Value::Null))
}

async fn breaker_snapshot(State(state): State<AppState>) -> Json<Vec<BreakerState>> {
    Json(state.breaker.lock().unwrap().snapshot())
}

async fn breaker_set_state(
    State(state): State<AppState>,
    Json(bs): Json<BreakerState>,
) -> Json<Value> {
    state.breaker.lock().unwrap().set_state(bs.clone());
    if let Ok(val) = serde_json::to_value(&bs) {
        state.emit("control:breakerState", val);
    }
    Json(serde_json::json!({ "ok": true }))
}

async fn hive_registry(State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    Json(reg)
}

async fn hive_patch_field(
    state: AppState,
    key: &str,
    req: PatchFieldReq,
) -> Result<Json<Value>, StatusCode> {
    state.sync_hive_home();
    let val = if req.value.is_null() {
        None
    } else {
        Some(req.value)
    };
    let ok = state
        .hive
        .lock()
        .unwrap()
        .patch_agent_field(&req.id, key, val)
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(serde_json::json!({ "ok": ok })))
}

async fn hive_patch_role(State(s): State<AppState>, Json(r): Json<PatchFieldReq>) -> Result<Json<Value>, StatusCode> {
    hive_patch_field(s, "role", r).await
}

async fn hive_patch_goal(State(s): State<AppState>, Json(r): Json<PatchFieldReq>) -> Result<Json<Value>, StatusCode> {
    hive_patch_field(s, "goal", r).await
}

async fn hive_rename_agent(State(s): State<AppState>, Json(r): Json<PatchFieldReq>) -> Result<Json<Value>, StatusCode> {
    hive_patch_field(s, "name", r).await
}

async fn hive_set_hold(State(s): State<AppState>, Json(r): Json<PatchFieldReq>) -> Result<Json<Value>, StatusCode> {
    let res = hive_patch_field(s.clone(), "onHold", r.clone()).await?;
    s.emit("hive:agentHold", serde_json::json!({ "id": r.id, "onHold": r.value }));
    Ok(res)
}

async fn hive_set_archived(State(s): State<AppState>, Json(r): Json<PatchFieldReq>) -> Result<Json<Value>, StatusCode> {
    let res = hive_patch_field(s.clone(), "archived", r.clone()).await?;
    if r.value.as_bool() == Some(true) {
        s.emit("hive:agentArchived", serde_json::json!({ "id": r.id }));
    }
    Ok(res)
}

async fn hive_board(State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    Json(serde_json::json!({ "board": state.hive.lock().unwrap().board() }))
}

async fn hive_tasks(State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    Json(state.hive.lock().unwrap().tasks())
}

async fn hive_tasks_archive(State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    Json(state.hive.lock().unwrap().tasks_archive())
}

async fn hive_add_task(
    State(state): State<AppState>,
    Json(task): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    state.sync_hive_home();
    let ok = state
        .hive
        .lock()
        .unwrap()
        .add_task(task)
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(serde_json::json!({ "ok": ok })))
}

async fn hive_patch_task(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(patch): Json<Map<String, Value>>,
) -> Result<Json<Value>, StatusCode> {
    state.sync_hive_home();
    let ok = state
        .hive
        .lock()
        .unwrap()
        .patch_task(&id, &patch)
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(serde_json::json!({ "ok": ok })))
}

async fn hive_delete_task(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    state.sync_hive_home();
    let ok = state
        .hive
        .lock()
        .unwrap()
        .delete_task(&id)
        .map_err(|_| StatusCode::CONFLICT)?;
    Ok(Json(serde_json::json!({ "ok": ok })))
}

async fn hive_log(State(state): State<AppState>, Query(q): Query<LogQuery>) -> Json<Vec<Value>> {
    state.sync_hive_home();
    Json(state.hive.lock().unwrap().log_tail(q.n.unwrap_or(200)))
}

async fn hive_inbox(Path(id): Path<String>, State(state): State<AppState>) -> Json<Vec<Value>> {
    state.sync_hive_home();
    Json(state.hive.lock().unwrap().inbox(&id))
}

async fn hive_messages(
    State(state): State<AppState>,
    Query(q): Query<MessagesQuery>,
) -> Json<Vec<Value>> {
    state.sync_hive_home();
    Json(
        state
            .hive
            .lock()
            .unwrap()
            .messages(q.agent_id.as_deref(), q.limit.unwrap_or(100)),
    )
}

async fn hive_search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Json<Value> {
    state.sync_hive_home();
    let hits = state
        .hive
        .lock()
        .unwrap()
        .text_search(q.q.as_deref().unwrap_or(""));
    Json(serde_json::to_value(hits).unwrap_or(Value::Array(Vec::new())))
}

async fn hive_context(Path(id): Path<String>, State(state): State<AppState>) -> Json<Value> {
    let ctx = state.hooks.context_by_id.lock().unwrap().get(&id).cloned();
    let model = state.hooks.model_by_id.lock().unwrap().get(&id).cloned();
    Json(serde_json::json!({ "context": ctx, "model": model }))
}

pub fn hive_router() -> Router<AppState> {
    Router::new()
        .route("/hook/{agent_id}", post(handle_hook))
        .route("/api/v1/control/pause", post(control_pause))
        .route("/api/v1/control/auto-delivery", post(control_auto_delivery))
        .route("/api/v1/control/resume", post(control_resume))
        .route("/api/v1/control/gate-tool", post(control_gate_tool))
        .route("/api/v1/control/steer", post(control_steer))
        .route("/api/v1/control/halt", post(control_halt))
        .route("/api/v1/control/unhalt", post(control_unhalt))
        .route("/api/v1/control/{id}/snapshot", get(control_snapshot))
        .route("/api/v1/breaker/snapshot", get(breaker_snapshot))
        .route("/api/v1/breaker/state", post(breaker_set_state))
        .route("/api/v1/hive/registry", get(hive_registry))
        .route("/api/v1/hive/agent/role", post(hive_patch_role))
        .route("/api/v1/hive/agent/goal", post(hive_patch_goal))
        .route("/api/v1/hive/agent/rename", post(hive_rename_agent))
        .route("/api/v1/hive/agent/hold", post(hive_set_hold))
        .route("/api/v1/hive/agent/archived", post(hive_set_archived))
        .route("/api/v1/hive/board", get(hive_board))
        .route("/api/v1/hive/tasks", get(hive_tasks).post(hive_add_task))
        .route("/api/v1/hive/tasks/archive", get(hive_tasks_archive))
        .route(
            "/api/v1/hive/tasks/{id}",
            patch(hive_patch_task).delete(hive_delete_task),
        )
        .route("/api/v1/hive/log", get(hive_log))
        .route("/api/v1/hive/inbox/{id}", get(hive_inbox))
        .route("/api/v1/hive/messages", get(hive_messages))
        .route("/api/v1/hive/search", get(hive_search))
        .route("/api/v1/hive/context/{id}", get(hive_context))
}
