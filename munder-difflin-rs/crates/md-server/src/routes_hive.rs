use crate::state::{AppState, WorkerRecord};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use md_core::config::expand_tilde;
use md_hive::breaker::BreakerState;
use md_pty::manager::SpawnOptions;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

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
    #[serde(alias = "limit")]
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HiveSendReq {
    msg: Value,
    from: Option<String>,
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

    let eff_agent_id = payload
        .get("agent_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&agent_id)
        .to_string();
    let event_name = payload
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let (goal, style, identity, roster_ctx) =
        crate::hive_bootstrap::build_hook_context_for_agent(&state, &eff_agent_id);

    let (mut reply, mut side) = state.hooks.handle_hook(
        &agent_id,
        &payload,
        goal.as_deref(),
        style.as_deref(),
        identity.as_deref(),
        roster_ctx.as_deref(),
    );

    if matches!(event_name, "Stop" | "SubagentStop") && reply.get("continue").is_none() {
        state.sync_hive_home();
        let unread = state.hive.lock().unwrap().inbox(&eff_agent_id);
        if !unread.is_empty() {
            let inbox_dir = state
                .harness_home()
                .map(|h| {
                    h.join("hive")
                        .join("agents")
                        .join(&eff_agent_id)
                        .join("inbox")
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_else(|| format!("agents/{eff_agent_id}/inbox"));
            reply = serde_json::json!({
                "decision": "block",
                "reason": format!(
                    "You have {} unread hive inbox message(s) in {}. Read and act on them, then move processed files to inbox/.done before stopping.",
                    unread.len(),
                    inbox_dir
                )
            });
            for (ch, ev) in &mut side.events {
                if ch == "hive:hookEvent"
                    && let Some(obj) = ev.as_object_mut()
                {
                    obj.insert("blocked".to_string(), Value::Bool(true));
                }
            }
        }
    }

    for (ch, ev) in side.events {
        state.emit(&ch, ev);
    }
    if matches!(
        event_name,
        "SessionStart" | "Stop" | "Status" | "PostToolUse" | "UserPromptSubmit"
    ) {
        crate::hive_bootstrap::sync_fleet_json(&state);
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
    let next_cfg = {
        let mut store = state.config.lock().unwrap();
        let mut map = store
            .read()
            .extra
            .get("autoDeliveryPausedAgents")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        if req.on {
            map.insert(req.agent_id.clone(), Value::Bool(true));
        } else {
            map.remove(&req.agent_id);
        }
        store
            .update(serde_json::json!({ "autoDeliveryPausedAgents": map }))
            .ok()
    };
    if let Some(cfg) = next_cfg
        && let Ok(val) = serde_json::to_value(&cfg)
    {
        state.emit("config:changed", val);
    }
    crate::hive_bootstrap::sync_fleet_json(&state);
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

fn deliver_or_enqueue(state: &AppState, target_id: &str, text: String, provider_hint: Option<&str>) {
    state.emit(
        "hive:enqueue",
        serde_json::json!({ "targetId": target_id, "text": text }),
    );
    if state.events_tx.receiver_count() == 0 {
        let pty_id = format!("pty-{target_id}");
        if state.pty.info(&pty_id).is_some() {
            let sub = state.submitter.clone();
            let st_clone = state.clone();
            let tid = target_id.to_string();
            let prov = provider_hint.unwrap_or("opencode").to_string();
            tokio::spawn(async move {
                let _ = sub
                    .submit(
                        md_pty::submitter::PtySubmitRequest {
                            pty_id,
                            agent_id: Some(tid),
                            provider: prov,
                            text,
                            source: "server-enqueue".to_string(),
                            settle_ms: Some(250),
                            press_only: false,
                            ref_id: None,
                        },
                        move |r| {
                            st_clone.emit("pty:began", serde_json::json!({ "ptyId": r.pty_id }));
                        },
                    )
                    .await;
            });
        }
    }
}

async fn hive_send(
    State(state): State<AppState>,
    Json(req): Json<HiveSendReq>,
) -> Json<Value> {
    state.sync_hive_home();
    let from = req.from.as_deref().unwrap_or("human");
    let send_res = state.hive.lock().unwrap().send_message(req.msg, from);
    match send_res {
        Ok(saved) => {
            state.emit("hive:message", saved.clone());
            let act = saved.get("act").and_then(|v| v.as_str()).unwrap_or("inform");
            let subject = saved.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let body = saved.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let to_str = saved.get("to").and_then(|v| v.as_str()).unwrap_or("");
            if to_str != "human"
                && let Some(targets) = saved.get("targets").and_then(|v| v.as_array())
            {
                let text = format!("[HIVE {act} from {from}] {subject}\n{body}")
                    .trim()
                    .to_string();
                for t in targets {
                    if let Some(tid) = t.as_str()
                        && tid != "human"
                    {
                        deliver_or_enqueue(&state, tid, text.clone(), None);
                    }
                }
            }
            crate::hive_bootstrap::sync_fleet_json(&state);
            Json(serde_json::json!({ "ok": true, "message": saved }))
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "error": e.to_string() })),
    }
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

async fn hive_directory(State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    let mut god_id: Option<String> = None;
    let mut rows = Vec::new();
    let contexts = state.hooks.context_by_id.lock().unwrap().clone();
    let models = state.hooks.model_by_id.lock().unwrap().clone();
    let breakers = state.breaker.lock().unwrap();
    let ptys = state.pty.list();

    if let Some(agents) = reg.get("agents").and_then(|v| v.as_object()) {
        for (id, info) in agents {
            let is_god = info.get("isGod").and_then(|v| v.as_bool()).unwrap_or(false) || id == "god";
            if is_god && god_id.is_none() {
                god_id = Some(id.clone());
            }
            let cwd = info.get("cwd").and_then(|v| v.as_str()).map(ToString::to_string);
            let cwd_valid = cwd.as_deref().map(|c| expand_tilde(c).is_dir());
            let ctx = contexts.get(id);
            let ctx_pct = ctx.and_then(|c| {
                if c.limit > 0 {
                    Some(((c.tokens as f64 / c.limit as f64) * 100.0).round() as u64)
                } else {
                    None
                }
            });
            let pty_running = ptys.iter().any(|p| p.id == format!("pty-{id}"));
            let stopped = info
                .get("stopped")
                .and_then(|v| v.as_bool())
                .unwrap_or(!pty_running);
            let inbox_backlog = state.hive.lock().unwrap().inbox(id).len();
            rows.push(serde_json::json!({
                "id": id,
                "name": info.get("name").and_then(|v| v.as_str()).unwrap_or(id),
                "role": info.get("role").and_then(|v| v.as_str()).unwrap_or(if is_god { "orchestrator" } else { "agent" }),
                "provider": info.get("provider").and_then(|v| v.as_str()).unwrap_or("claude"),
                "model": models.get(id).cloned().or_else(|| info.get("model").and_then(|v| v.as_str()).map(ToString::to_string)),
                "status": info.get("status").and_then(|v| v.as_str()).unwrap_or("idle"),
                "cwd": cwd,
                "cwdValid": cwd_valid,
                "archived": info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false),
                "stopped": stopped,
                "isGod": is_god,
                "inboxBacklog": inbox_backlog,
                "breaker": breakers.state_for(id).level,
                "contextTokens": ctx.map(|c| c.tokens),
                "contextLimit": ctx.map(|c| c.limit),
                "contextPct": ctx_pct,
            }));
        }
    }
    Json(serde_json::json!({ "godId": god_id, "agents": rows }))
}

async fn hive_human_mail_since(
    State(state): State<AppState>,
    Json(since): Json<HashMap<String, i64>>,
) -> Json<Value> {
    state.sync_hive_home();
    let res = state.hive.lock().unwrap().human_mail_since(&since);
    Json(res)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn provider_default_command(provider: &str, model: Option<&str>) -> (String, Vec<String>) {
    let mut args = Vec::new();
    let bin = match provider {
        "codex" => "codex",
        "gemini" => "gemini",
        "opencode" => {
            args.push("--auto".to_string());
            "opencode"
        }
        "cursor" => "agent",
        "antigravity" => "agy",
        "grok" => "grok",
        "kimi" => "kimi",
        "qwen" => "qwen",
        "crush" => "crush",
        "pi" => "pi",
        "copilot" => "copilot",
        _ => "claude",
    };
    if let Some(m) = model.map(str::trim).filter(|s| !s.is_empty())
        && matches!(
            provider,
            "opencode" | "claude" | "codex" | "gemini" | "cursor" | "grok" | "kimi" | "antigravity" | "qwen" | "crush" | "pi" | "copilot"
        )
    {
        args.push("--model".to_string());
        args.push(m.to_string());
    }
    (bin.to_string(), args)
}

fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() || n == '~' {
                        break;
                    }
                }
            } else if matches!(chars.peek(), Some(']')) {
                chars.next();
                for n in chars.by_ref() {
                    if n == '\x07' {
                        break;
                    }
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

fn worker_tokens_used(state: &AppState, worker_id: &str, cwd: &str) -> u64 {
    if let Some(ctx) = state.hooks.context_by_id.lock().unwrap().get(worker_id) {
        return ctx.tokens;
    }
    let u = md_hive::usage::read_agent_usage_in(&md_hive::usage::default_projects_root(), cwd, None);
    u.input_tokens + u.output_tokens + u.cache_write_tokens + u.cache_read_tokens
}

fn extract_final_response(state: &AppState, pty_id: &str) -> Value {
    let tail = state.pty.tail(pty_id);
    if tail.trim().is_empty() {
        return Value::Null;
    }
    let clean = strip_ansi(&tail);
    let lines: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Value::Null;
    }
    let start = lines.len().saturating_sub(12);
    let joined = lines[start..].join("\n");
    let capped: String = joined.chars().take(1200).collect();
    Value::String(capped)
}

fn persist_workers_history(state: &AppState, history: &[Value]) {
    let path = state.data_dir.join("workers-history.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(history) {
        let _ = std::fs::write(path, text);
    }
}

fn worker_history_row(
    w: &WorkerRecord,
    result: &str,
    ended_at: u64,
    tokens_used: u64,
    final_response: Value,
) -> Value {
    serde_json::json!({
        "id": format!("{}-{}", w.worker_id, ended_at),
        "workerId": w.worker_id,
        "reqId": w.req_id,
        "name": w.name,
        "job": w.job,
        "firstPrompt": w.job,
        "finalResponse": final_response,
        "cwd": w.cwd,
        "baseBranch": w.base_branch,
        "spawnedAt": w.started_at,
        "endedAt": ended_at,
        "tokensUsed": tokens_used,
        "tokenCap": w.token_cap,
        "result": result,
        "worktree": w.worktree,
        "hasSlack": w.has_slack,
        "model": w.model,
        "effort": w.effort,
    })
}

pub(crate) fn prune_exited_workers(state: &AppState) {
    let ptys = state.pty.list();
    let mut workers = state.workers.lock().unwrap();
    let mut history = state.workers_history.lock().unwrap();
    let now = now_ms();
    let mut exited_ids = Vec::new();
    workers.retain(|w| {
        if ptys.iter().any(|p| p.id == w.pty_id) {
            true
        } else {
            let tokens = worker_tokens_used(state, &w.worker_id, &w.cwd);
            let final_resp = extract_final_response(state, &w.pty_id);
            history.push(worker_history_row(w, "exited", now, tokens, final_resp));
            exited_ids.push(w.worker_id.clone());
            false
        }
    });
    if !exited_ids.is_empty() {
        if history.len() > 200 {
            let drain_n = history.len() - 200;
            history.drain(..drain_n);
        }
        persist_workers_history(state, &history);
        state.sync_hive_home();
        {
            let mut hive = state.hive.lock().unwrap();
            for aid in &exited_ids {
                let _ = hive.patch_agent_field(aid, "sleeping", Some(Value::Bool(false)));
                let _ = hive.patch_agent_field(aid, "stopped", Some(Value::Bool(true)));
                let _ = hive.patch_agent_field(
                    aid,
                    "status",
                    Some(Value::String("exited".to_string())),
                );
            }
        }
        crate::hive_bootstrap::sync_fleet_json(state);
    }
}

async fn workers_list(State(state): State<AppState>) -> Json<Value> {
    prune_exited_workers(&state);
    let ptys = state.pty.list();
    let now = now_ms();
    let max_workers = {
        let mut cfg = state.config.lock().unwrap();
        let c = cfg.read();
        c.extra
            .get("maxConcurrentWorkers")
            .or_else(|| c.extra.get("maxWorkers"))
            .and_then(|v| v.as_u64())
            .unwrap_or(4)
    };
    let live_records = state.workers.lock().unwrap().clone();
    let live_ids: std::collections::HashSet<String> =
        live_records.iter().map(|w| w.worker_id.clone()).collect();
    let live: Vec<Value> = live_records
        .iter()
        .map(|w| {
            let pty_entry = ptys.iter().find(|p| p.id == w.pty_id);
            let idle_ms = pty_entry.map(|p| now.saturating_sub(p.last_output_at));
            let tokens_used = worker_tokens_used(&state, &w.worker_id, &w.cwd);
            serde_json::json!({
                "workerId": w.worker_id,
                "ptyId": w.pty_id,
                "name": w.name,
                "job": w.job,
                "model": w.model,
                "cwd": w.cwd,
                "baseBranch": w.base_branch,
                "ageMs": now.saturating_sub(w.started_at),
                "idleMs": idle_ms,
                "tokensUsed": tokens_used,
                "tokenCap": w.token_cap,
                "hasSlack": w.has_slack,
                "releasing": w.releasing,
                "worktree": w.worktree,
                "effort": w.effort,
            })
        })
        .collect();

    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    let mut preserved = Vec::new();
    if let Some(agents) = reg.get("agents").and_then(|v| v.as_object()) {
        for (aid, info) in agents {
            let is_temp = info.get("isTemp").and_then(|v| v.as_bool()).unwrap_or(false)
                || aid.starts_with("worker-");
            let archived = info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false);
            if is_temp && !archived && !live_ids.contains(aid) {
                preserved.push(serde_json::json!({
                    "workerId": aid,
                    "name": info.get("name").and_then(|v| v.as_str()).unwrap_or(aid),
                    "job": info.get("role").and_then(|v| v.as_str()).unwrap_or(""),
                    "cwd": info.get("cwd").and_then(|v| v.as_str()).unwrap_or(""),
                    "provider": info.get("provider").and_then(|v| v.as_str()).unwrap_or("claude"),
                    "status": info.get("status").and_then(|v| v.as_str()).unwrap_or("sleeping"),
                }));
            }
        }
    }

    Json(serde_json::json!({
        "live": live,
        "preserved": preserved,
        "maxWorkers": max_workers,
    }))
}

async fn workers_history(State(state): State<AppState>) -> Json<Vec<Value>> {
    prune_exited_workers(&state);
    Json(state.workers_history.lock().unwrap().clone())
}

async fn worker_stop(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<Value> {
    let worker = {
        let mut workers = state.workers.lock().unwrap();
        let pos = workers.iter().position(|w| w.worker_id == id);
        pos.map(|idx| workers.remove(idx))
    };
    let Some(w) = worker else {
        return Json(serde_json::json!({ "ok": false, "error": "not found" }));
    };
    let tokens = worker_tokens_used(&state, &w.worker_id, &w.cwd);
    let final_resp = extract_final_response(&state, &w.pty_id);
    let _ = state.pty.kill(&w.pty_id);
    {
        state.sync_hive_home();
        let _ = state.hive.lock().unwrap().patch_agent_field(
            &w.worker_id,
            "archived",
            Some(Value::Bool(true)),
        );
    }
    {
        let mut history = state.workers_history.lock().unwrap();
        history.push(worker_history_row(&w, "stopped", now_ms(), tokens, final_resp));
        if history.len() > 200 {
            let drain_n = history.len() - 200;
            history.drain(..drain_n);
        }
        persist_workers_history(&state, &history);
    }
    state.emit("hive:agentArchived", serde_json::json!({ "id": w.worker_id }));
    Json(serde_json::json!({ "ok": true }))
}

pub(crate) fn process_spawn_requests(state: &AppState) {
    let Some(home) = state.harness_home() else {
        return;
    };
    let (allow_temps, max_workers, max_temps_per_agent, default_provider, default_subagent_model) = {
        let mut cfg_store = state.config.lock().unwrap();
        let cfg = cfg_store.read();
        let allow = cfg.orchestrator_may_spawn
            && cfg
                .extra
                .get("allowTemps")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
        let max_w = cfg
            .extra
            .get("maxConcurrentWorkers")
            .or_else(|| cfg.extra.get("maxWorkers"))
            .and_then(|v| v.as_u64())
            .unwrap_or(4) as usize;
        let max_per_agent = cfg
            .extra
            .get("maxTempsPerAgent")
            .and_then(|v| v.as_u64())
            .unwrap_or(2) as usize;
        let prov = cfg
            .extra
            .get("godProvider")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("claude")
            .to_string();
        let sub_model = cfg
            .extra
            .get("subAgentDefaultModel")
            .or_else(|| cfg.extra.get("defaultModel"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        (allow, max_w, max_per_agent, prov, sub_model)
    };
    if !allow_temps || max_workers == 0 {
        return;
    }

    let hive_root = home.join("hive");
    let projects_root = hive_root.join("projects");
    let mut dirs = vec![(hive_root.join("spawn-requests"), "god".to_string())];
    if let Ok(agent_dirs) = std::fs::read_dir(hive_root.join("agents")) {
        for entry in agent_dirs.flatten() {
            let sr = entry.path().join("spawn-requests");
            if sr.is_dir() {
                dirs.push((sr, entry.file_name().to_string_lossy().into_owned()));
            }
        }
    }

    for (dir, requester) in dirs {
        // Non-god agents are capped by `maxTempsPerAgent` (`0` disables agent-initiated temps).
        if requester != "god" && max_temps_per_agent == 0 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        files.sort();

        for path in files {
            prune_exited_workers(state);
            if state.workers.lock().unwrap().len() >= max_workers {
                return;
            }
            if requester != "god"
                && state
                    .workers
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|w| w.requested_by == requester)
                    .count()
                    >= max_temps_per_agent
            {
                break;
            }
            let fname = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "req.json".to_string());
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let fail_file = |reason: &str| {
                let failed_dir = dir.join(".failed");
                let _ = std::fs::create_dir_all(&failed_dir);
                let _ = std::fs::write(
                    failed_dir.join(&fname),
                    serde_json::json!({ "error": reason, "raw": raw }).to_string(),
                );
                let _ = std::fs::remove_file(&path);
            };
            let Ok(req) = serde_json::from_str::<Value>(&raw) else {
                fail_file("invalid JSON in spawn request");
                continue;
            };
            let objective = req
                .get("objective")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            let cwd_raw = req
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            if objective.is_empty() || cwd_raw.is_empty() {
                fail_file("missing required 'objective' or 'cwd'");
                continue;
            }
            let cwd_path = expand_tilde(cwd_raw);
            if cwd_path.starts_with(&projects_root) && !cwd_path.exists() {
                let _ = std::fs::create_dir_all(&cwd_path);
            }
            if !cwd_path.is_dir() {
                fail_file(&format!("cwd does not exist: {}", cwd_path.display()));
                continue;
            }
            let base_slug: String = fname
                .trim_end_matches(".json")
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                .collect();
            let mut worker_id = format!("worker-{base_slug}");
            let mut pty_id = format!("pty-{worker_id}");
            let mut n = 2usize;
            while state.pty.info(&pty_id).is_some()
                || state.workers.lock().unwrap().iter().any(|w| w.worker_id == worker_id)
            {
                worker_id = format!("worker-{base_slug}-{n}");
                pty_id = format!("pty-{worker_id}");
                n += 1;
            }
            let slug = worker_id
                .strip_prefix("worker-")
                .unwrap_or(&worker_id)
                .to_string();
            let provider = req
                .get("provider")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&default_provider)
                .to_string();
            let model_opt = req
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or(default_subagent_model.as_deref());
            let (cmd_bin, cmd_args, command_str) = match req
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(raw_cmd) => {
                    let mut parts = raw_cmd.split_whitespace();
                    let bin = parts.next().unwrap_or("claude").to_string();
                    let args: Vec<String> = parts.map(ToString::to_string).collect();
                    (bin, args, raw_cmd.to_string())
                }
                None => {
                    let (bin, args) = provider_default_command(&provider, model_opt);
                    let full = if args.is_empty() {
                        bin.clone()
                    } else {
                        format!("{bin} {}", args.join(" "))
                    };
                    (bin, args, full)
                }
            };
            let name = req
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&worker_id)
                .to_string();

            let mut spawn_opts = SpawnOptions {
                id: pty_id.clone(),
                cwd: cwd_path.to_string_lossy().into_owned(),
                command: cmd_bin,
                args: cmd_args,
                cols: Some(100),
                rows: Some(30),
                env: HashMap::new(),
                shell_script: None,
                provider: Some(provider.clone()),
                resume: Some(false),
                hive: None,
            };
            let _ = crate::hive_bootstrap::prepare_agent_spawn(
                state,
                &mut spawn_opts,
                &worker_id,
                &name,
                &provider,
                false,
                Some(objective),
            );
            let spawn_res = state.pty.spawn(spawn_opts);
            if !spawn_res.ok {
                fail_file(spawn_res.error.as_deref().unwrap_or("spawn failed"));
                continue;
            }

            let done_dir = dir.join(".done");
            let _ = std::fs::create_dir_all(&done_dir);
            let _ = std::fs::rename(&path, done_dir.join(&fname));

            let mut entry = Map::new();
            entry.insert("id".to_string(), Value::String(worker_id.clone()));
            entry.insert("name".to_string(), Value::String(name.clone()));
            entry.insert("role".to_string(), Value::String(objective.to_string()));
            entry.insert("provider".to_string(), Value::String(provider.clone()));
            entry.insert("cwd".to_string(), Value::String(cwd_path.to_string_lossy().into_owned()));
            entry.insert("isTemp".to_string(), Value::Bool(true));
            entry.insert("archived".to_string(), Value::Bool(false));
            entry.insert("sleeping".to_string(), Value::Bool(false));
            entry.insert("stopped".to_string(), Value::Bool(false));
            entry.insert("status".to_string(), Value::String("working".to_string()));

            {
                let mut hive = state.hive.lock().unwrap();
                let _ = hive.register_worker(&worker_id, entry);
                let _ = hive.send_message(
                    serde_json::json!({
                        "to": worker_id,
                        "act": "request",
                        "subject": "Temp objective",
                        "body": objective,
                    }),
                    &requester,
                );
                hive.append_log(&serde_json::json!({
                    "kind": "spawn",
                    "agentId": worker_id,
                    "name": name,
                    "temp": true,
                }));
            }

            let started_at = now_ms();
            state.workers.lock().unwrap().push(WorkerRecord {
                worker_id: worker_id.clone(),
                req_id: slug,
                pty_id: pty_id.clone(),
                name: name.clone(),
                job: objective.to_string(),
                cwd: cwd_path.to_string_lossy().into_owned(),
                base_branch: req
                    .get("baseBranch")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main")
                    .to_string(),
                provider: provider.clone(),
                model: req.get("model").and_then(|v| v.as_str()).map(ToString::to_string),
                command: command_str.clone(),
                started_at,
                token_cap: req.get("tokenCap").and_then(|v| v.as_u64()),
                has_slack: req.get("slack").is_some(),
                releasing: false,
                worktree: req
                    .get("worktree")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                effort: req
                    .get("effort")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                requested_by: requester.clone(),
            });

            crate::hive_bootstrap::sync_fleet_json(state);
            state.emit(
                "hive:agentSpawned",
                serde_json::json!({
                    "id": worker_id,
                    "ptyId": pty_id,
                    "name": name,
                    "cwd": cwd_path.to_string_lossy().into_owned(),
                    "command": command_str,
                    "provider": provider,
                    "model": req.get("model").cloned().unwrap_or(Value::Null),
                    "character": req.get("character").cloned().unwrap_or(Value::Null),
                    "accent": req.get("accent").cloned().unwrap_or(Value::Null),
                    "role": objective,
                }),
            );
            let hr_str = hive_root.to_string_lossy().into_owned();
            let cwd_disp = cwd_path.to_string_lossy().into_owned();
            let job_prompt = format!(
                "[TEMP JOB from {requester}] {objective}\nYour workspace: `{cwd_disp}`\nYour AGENT_ID is `{worker_id}`. Read `{hr_str}/COMMANDS.md` if needed.\nWhen finished, write ONE JSON file to `{hr_str}/agents/{worker_id}/outbox/done-{worker_id}.json` containing:\n{{\"to\":\"{requester}\",\"act\":\"done\",\"subject\":\"Result\",\"body\":\"<concise summary of your findings/files written>\"}}"
            );
            let st_delayed = state.clone();
            let tid_delayed = worker_id.clone();
            let prov_delayed = provider.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
                deliver_or_enqueue(&st_delayed, &tid_delayed, job_prompt, Some(&prov_delayed));
            });
        }
    }
}

fn evaluate_circuit_breaker_tick(state: &AppState) {
    let usage_rows = crate::routes_usage_memory_kg::collect_agent_usage_rows(state);
    if usage_rows.is_empty() {
        return;
    }
    let mut caps = HashMap::new();
    {
        let mut cfg_store = state.config.lock().unwrap();
        let cfg = cfg_store.read();
        if let Some(limits) = cfg.extra.get("tokenLimits").and_then(|v| v.as_object()) {
            for (aid, entry) in limits {
                if aid == "office" {
                    continue;
                }
                let cap_opt = entry
                    .as_u64()
                    .or_else(|| entry.get("cap").and_then(|c| c.as_u64()));
                if let Some(c) = cap_opt.filter(|&c| c > 0) {
                    caps.insert(aid.clone(), c);
                }
            }
        }
    }
    for w in state.workers.lock().unwrap().iter() {
        if let Some(tc) = w.token_cap.filter(|&c| c > 0) {
            caps.entry(w.worker_id.clone()).or_insert(tc);
        }
    }
    let inputs: Vec<md_hive::breaker::BreakerUsageInput> = usage_rows
        .into_iter()
        .map(|(agent_id, u)| md_hive::breaker::BreakerUsageInput {
            agent_id,
            work_tokens: u
                .input_tokens
                .saturating_add(u.output_tokens)
                .saturating_add(u.cache_write_tokens),
            output_tokens: u.output_tokens,
            usd: u.estimated_cost_usd,
        })
        .collect();

    let decisions = state
        .breaker
        .lock()
        .unwrap()
        .tick_spend_and_velocity(&inputs, &caps, now_ms());

    for dec in decisions {
        if (dec.changed || dec.action != md_hive::breaker::BreakerAction::None)
            && let Ok(val) = serde_json::to_value(&dec.state)
        {
            state.emit("control:breakerState", val);
        }
        match dec.action {
            md_hive::breaker::BreakerAction::Stop => {
                state.control.lock().unwrap().halt(&dec.state.agent_id);
                let _ = state.pty.kill(&format!("pty-{}", dec.state.agent_id));
            }
            md_hive::breaker::BreakerAction::Steer | md_hive::breaker::BreakerAction::Constrain => {
                deliver_or_enqueue(
                    state,
                    &dec.state.agent_id,
                    format!("[CIRCUIT BREAKER {:?}] {}", dec.state.level, dec.state.reason),
                    None,
                );
            }
            md_hive::breaker::BreakerAction::None => {}
        }
    }
}

pub fn spawn_hive_poller(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(750));
        let mut ticks: u64 = 0;
        loop {
            interval.tick().await;
            ticks += 1;
            state.sync_hive_home();
            let routed = state.hive.lock().unwrap().drain_outboxes();
            for msg in routed {
                state.emit("hive:message", msg.clone());
                let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("agent");
                let act = msg.get("act").and_then(|v| v.as_str()).unwrap_or("inform");
                let subject = msg.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                let body = msg.get("body").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(targets) = msg.get("targets").and_then(|v| v.as_array()) {
                    for t in targets {
                        if let Some(tid) = t.as_str() {
                            let text = format!("[HIVE {act} from {from}] {subject}\n{body}")
                                .trim()
                                .to_string();
                            deliver_or_enqueue(&state, tid, text, None);
                        }
                    }
                }
            }
            process_spawn_requests(&state);
            prune_exited_workers(&state);
            if ticks == 1 || ticks.is_multiple_of(8) {
                evaluate_circuit_breaker_tick(&state);
                crate::hive_bootstrap::sync_fleet_json(&state);
            }
        }
    });
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WakeReq {
    agent_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScreenLinesReq {
    pty_id: String,
    #[serde(default = "default_screen_lines")]
    lines: usize,
}

fn default_screen_lines() -> usize {
    40
}

async fn hive_wake(State(state): State<AppState>, Json(req): Json<WakeReq>) -> Json<Value> {
    state.sync_hive_home();
    let pty_id = format!("pty-{}", req.agent_id);
    let agent_info = {
        let mut hive = state.hive.lock().unwrap();
        let _ = hive.patch_agent_field(&req.agent_id, "sleeping", Some(Value::Bool(false)));
        let _ = hive.patch_agent_field(&req.agent_id, "stopped", Some(Value::Bool(false)));
        let _ = hive.patch_agent_field(
            &req.agent_id,
            "status",
            Some(Value::String("idle".to_string())),
        );
        hive.registry()
            .get("agents")
            .and_then(|a| a.get(&req.agent_id))
            .and_then(|v| v.as_object())
            .cloned()
    };

    if state.pty.info(&pty_id).is_none()
        && let Some(ref info) = agent_info
    {
        let cwd = info
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| state.harness_home().map(|h| h.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let provider = info
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("opencode")
            .to_string();
        let model = info.get("model").and_then(|v| v.as_str());
        let name = info
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(&req.agent_id)
            .to_string();
        let role = info.get("role").and_then(|v| v.as_str());
        let (cmd_bin, cmd_args) = provider_default_command(&provider, model);
        let cmd_str = if cmd_args.is_empty() {
            cmd_bin.clone()
        } else {
            format!("{cmd_bin} {}", cmd_args.join(" "))
        };
        let mut spawn_opts = SpawnOptions {
            id: pty_id.clone(),
            cwd: cwd.clone(),
            command: cmd_bin,
            args: cmd_args,
            cols: Some(100),
            rows: Some(30),
            env: HashMap::new(),
            shell_script: None,
            provider: Some(provider.clone()),
            resume: Some(false),
            hive: None,
        };
        let seed = crate::hive_bootstrap::prepare_agent_spawn(
            &state,
            &mut spawn_opts,
            &req.agent_id,
            &name,
            &provider,
            false,
            role,
        );
        let res = state.pty.spawn(spawn_opts);
        if res.ok {
            state.emit(
                "hive:agentSpawned",
                serde_json::json!({
                    "id": req.agent_id,
                    "ptyId": pty_id,
                    "name": name,
                    "cwd": cwd,
                    "command": cmd_str,
                    "provider": provider,
                    "role": role,
                }),
            );
            if let Some(s) = seed {
                let st_clone = state.clone();
                let aid = req.agent_id.clone();
                let prov = provider.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                    deliver_or_enqueue(&st_clone, &aid, s, Some(&prov));
                });
            }
        }
    }

    let inbox = state.hive.lock().unwrap().inbox(&req.agent_id);
    if !inbox.is_empty() {
        let p = state
            .harness_home()
            .map(|h| h.join("hive/agents").join(&req.agent_id).join("inbox"))
            .map(|d| d.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = format!(
            "You have {} unread hive message(s) in {}. Please read, act on them, and move processed files to inbox/.done.",
            inbox.len(),
            p
        );
        deliver_or_enqueue(&state, &req.agent_id, text, None);
    }
    crate::hive_bootstrap::sync_fleet_json(&state);
    Json(serde_json::json!({ "ok": true }))
}

async fn hive_wake_text(Path(id): Path<String>, State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    let inbox = state.hive.lock().unwrap().inbox(&id);
    if inbox.is_empty() {
        return Json(Value::Null);
    }
    let text = inbox
        .iter()
        .map(|m| {
            let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("agent");
            let subj = m.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            let body = m.get("body").and_then(|v| v.as_str()).unwrap_or("");
            format!("[from {from}] {subj} {body}").trim().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    Json(Value::String(text))
}

async fn hive_wake_unconfirmed(Path(_id): Path<String>) -> Json<Value> {
    Json(Value::Array(Vec::new()))
}

async fn pty_screen_lines(
    State(state): State<AppState>,
    Json(req): Json<ScreenLinesReq>,
) -> Json<Value> {
    let tail = state.pty.tail(&req.pty_id);
    let clean = strip_ansi(&tail);
    let mut lines: Vec<&str> = clean.lines().map(str::trim_end).collect();
    let max_n = req.lines.clamp(1, 500);
    if lines.len() > max_n {
        lines = lines[lines.len() - max_n..].to_vec();
    }
    Json(serde_json::json!({ "ok": true, "lines": lines }))
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
        .route("/api/v1/hive/send", post(hive_send))
        .route("/api/v1/hive/inbox/{id}", get(hive_inbox))
        .route("/api/v1/hive/messages", get(hive_messages))
        .route("/api/v1/hive/search", get(hive_search))
        .route("/api/v1/hive/context/{id}", get(hive_context))
        .route("/api/v1/hive/directory", get(hive_directory))
        .route("/api/v1/hive/human-mail-since", post(hive_human_mail_since))
        .route("/api/v1/hive/wake", post(hive_wake))
        .route("/api/v1/hive/wake-retry", post(hive_wake))
        .route("/api/v1/hive/wake-text/{id}", get(hive_wake_text))
        .route("/api/v1/hive/wake-unconfirmed/{id}", get(hive_wake_unconfirmed))
        .route("/api/v1/pty/screen-lines", post(pty_screen_lines))
        .route("/api/v1/workers", get(workers_list))
        .route("/api/v1/workers/history", get(workers_history))
        .route("/api/v1/workers/{id}/stop", post(worker_stop))
}
