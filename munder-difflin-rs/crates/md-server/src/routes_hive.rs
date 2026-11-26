use crate::state::{AppState, WorkerRecord};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, patch, post},
};
use md_core::config::expand_tilde;
use md_hive::{
    breaker::BreakerState,
    manager::{hive_project_top_of, in_hive_projects, project_folder_name},
    switch_clear::{
        ClearGate, HookFacts, SwitchClearRule, WakeFacts, WakeStep, clear_command_for_provider,
        compaction_command_for_provider, handoff_request_text, is_compaction_command,
        oldest_waking_work, size_handoff_request_text, switch_clear_command, typed_kind,
        with_cleared_lead,
    },
};
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
    let event_name = md_hive::hooks::normalize_hook_event(
        payload
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown"),
    );
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

    let now = now_ms();
    state.switch_clear.lock().unwrap().note_hook(&HookFacts {
        agent_id: eff_agent_id.clone(),
        event: event_name.to_string(),
        session_id: payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        tool: payload
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        tool_use_id: payload
            .get("tool_use_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        subagent_id: payload
            .get("subagent_id")
            .or_else(|| payload.get("agent_id"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        at_ms: now,
    });

    let source = payload.get("source").and_then(|v| v.as_str());
    let seen_prompts = state
        .hooks
        .proof
        .lock()
        .unwrap()
        .recent_prompts(&eff_agent_id);
    let to_file = state.wake_filing.lock().unwrap().note_hook(
        &eff_agent_id,
        event_name,
        source,
        &seen_prompts,
        now,
    );
    if !to_file.is_empty() {
        state.sync_hive_home();
        let _ = state
            .hive
            .lock()
            .unwrap()
            .file_inbox_messages(&eff_agent_id, &to_file);
    }

    let mut stop_blocked = false;
    if matches!(event_name, "Stop" | "SubagentStop") && reply.get("continue").is_none() {
        state.sync_hive_home();
        let (unread, supports_blocking_stop) = {
            let mut hive = state.hive.lock().unwrap();
            let reg = hive.registry();
            let provider = reg
                .get("agents")
                .and_then(|a| a.get(&eff_agent_id))
                .and_then(|info| info.get("provider"))
                .and_then(|v| v.as_str())
                .unwrap_or("claude");
            (
                hive.inbox(&eff_agent_id),
                matches!(provider, "claude" | "codex" | "grok"),
            )
        };
        if supports_blocking_stop && !unread.is_empty() {
            stop_blocked = true;
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

    let next_reg_status = match event_name {
        "PreToolUse" | "PostToolUse" | "UserPromptSubmit" | "PreCompact" | "PostCompact" => {
            Some("working")
        }
        "Stop" | "PostInvocation" if !stop_blocked => Some("idle"),
        _ => None,
    };
    if let Some(st) = next_reg_status {
        let mut hive = state.hive.lock().unwrap();
        let _ = hive.patch_agent_field(&eff_agent_id, "status", Some(Value::String(st.to_string())));
    }

    for (ch, ev) in side.events {
        state.emit(&ch, ev);
    }
    if matches!(
        event_name,
        "SessionStart" | "Stop" | "PreCompact" | "PostCompact" | "UserPromptSubmit"
    ) {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "hook-seen",
            "event": event_name,
            "agentId": eff_agent_id,
        }));
    }
    if matches!(
        event_name,
        "SessionStart" | "Stop" | "Status" | "PostToolUse" | "PreToolUse" | "UserPromptSubmit"
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

pub(crate) fn deliver_or_enqueue(
    state: &AppState,
    target_id: &str,
    text: String,
    provider_hint: Option<&str>,
) {
    deliver_or_enqueue_inner(state, target_id, text, provider_hint, false);
}

fn deliver_or_enqueue_inner(
    state: &AppState,
    target_id: &str,
    text: String,
    provider_hint: Option<&str>,
    from_inbox: bool,
) {
    let is_archived = target_id != "god"
        && (state.ended_agents.lock().unwrap().contains_key(target_id)
            || state
                .hive
                .lock()
                .unwrap()
                .registry()
                .get("agents")
                .and_then(|a| a.get(target_id))
                .and_then(|info| info.get("archived"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false));
    if is_archived {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "archived-recipient",
            "to": target_id,
            "fromInbox": from_inbox,
        }));
        return;
    }
    let payload = if from_inbox {
        serde_json::json!({ "targetId": target_id, "text": text, "fromInbox": true })
    } else {
        serde_json::json!({ "targetId": target_id, "text": text })
    };
    state.emit("hive:enqueue", payload);
    if state.events_tx.receiver_count() == 0 {
        let pty_id = format!("pty-{target_id}");
        if state.pty.info(&pty_id).is_some() {
            let sub = state.submitter.clone();
            let st_clone = state.clone();
            let st_done = state.clone();
            let tid = target_id.to_string();
            let tid_done = target_id.to_string();
            let text_done = text.clone();
            let prov = provider_hint.unwrap_or("opencode").to_string();
            let st_hooks = st_done.clone();
            let st_probe = st_done.clone();
            tokio::spawn(async move {
                let probe = AppScreenProbe::new(&st_probe);
                let hooks = AppSubmitHooks(&st_hooks);
                if let Ok(res) = sub
                    .submit_with_context(
                        md_pty::submitter::PtySubmitRequest {
                            pty_id,
                            agent_id: Some(tid),
                            provider: prov,
                            text,
                            source: "server-enqueue".to_string(),
                            settle_ms: Some(250),
                            press_only: false,
                            idle: Some(true),
                            ref_id: None,
                        },
                        Some(&probe),
                        &hooks,
                        move |r| {
                            st_clone.emit("pty:began", serde_json::json!(r.pty_id));
                        },
                    )
                    .await
                {
                    note_agent_prompt_submitted(
                        &st_done,
                        &tid_done,
                        &text_done,
                        &res.outcome,
                        res.ms,
                        false,
                        None,
                    );
                }
            });
        }
    }
}

pub(crate) fn note_agent_prompt_submitted(
    state: &AppState,
    agent_id: &str,
    text: &str,
    outcome: &str,
    elapsed_ms: u64,
    press_only: bool,
    provider_hint: Option<&str>,
) {
    let now = now_ms();
    let to_file = state.wake_filing.lock().unwrap().note_typed(
        agent_id,
        text,
        outcome,
        elapsed_ms,
        press_only,
        now,
    );
    if !to_file.is_empty() {
        state.sync_hive_home();
        let _ = state
            .hive
            .lock()
            .unwrap()
            .file_inbox_messages(agent_id, &to_file);
    }
    if state.asked_compact.lock().unwrap().note_typed(
        Some(agent_id),
        is_compaction_command(text),
        outcome,
        press_only,
    ) {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "compact-request-done",
            "agentId": agent_id,
        }));
    }
    let provider = provider_hint
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let reg = state.hive.lock().unwrap().registry();
            reg.get("agents")
                .and_then(|a| a.get(agent_id))
                .and_then(|v| v.get("provider"))
                .and_then(|v| v.as_str())
                .unwrap_or("claude")
                .to_string()
        });
    if state.harness_home().is_some() {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "pty-submit",
            "agentId": agent_id,
            "provider": provider,
            "chars": text.len(),
            "pressOnly": press_only,
            "outcome": outcome,
            "ms": elapsed_ms,
        }));
    }
    let kind = typed_kind(text, switch_clear_command(&provider));
    let wake_work = if kind == md_hive::switch_clear::TypedKind::Wake {
        state.sync_hive_home();
        let inbox = state.hive.lock().unwrap().inbox(agent_id);
        oldest_waking_work(&inbox)
    } else {
        None
    };
    state.switch_clear.lock().unwrap().note_typed(
        Some(agent_id),
        kind,
        outcome,
        press_only,
        wake_work,
        now,
    );
    if let Some(rotated_bytes) =
        state
            .codex_size
            .lock()
            .unwrap()
            .note_typed(Some(agent_id), kind, outcome, press_only)
    {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "codex-session-rotated",
            "agentId": agent_id,
            "bytes": rotated_bytes,
        }));
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
                        deliver_or_enqueue_inner(&state, tid, text.clone(), None, true);
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
    let mut ctx = state.hooks.context_by_id.lock().unwrap().get(&id).cloned();
    let mut model = state.hooks.model_by_id.lock().unwrap().get(&id).cloned();
    if ctx.is_none() {
        state.sync_hive_home();
        let reg = state.hive.lock().unwrap().registry();
        if let Some(info) = reg.get("agents").and_then(|v| v.get(&id))
            && info.get("provider").and_then(|v| v.as_str()) == Some("opencode")
        {
            let cwd_str = info.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
            let expanded = if cwd_str.is_empty() {
                String::new()
            } else {
                expand_tilde(cwd_str).to_string_lossy().into_owned()
            };
            let hook_sid = state.hooks.session_by_id.lock().unwrap().get(&id).cloned();
            let started_at = info.get("startedAt").and_then(|v| v.as_u64());
            let sleeping = info.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false);
            if (!expanded.is_empty() || hook_sid.is_some())
                && let Some((oc_u, Some(tok))) = crate::routes_usage_memory_kg::read_opencode_usage_for_since(
                    &expanded,
                    hook_sid.as_deref(),
                    !sleeping,
                    started_at,
                )
            {
                let m = model
                    .clone()
                    .or(oc_u.model)
                    .or_else(|| info.get("model").and_then(|v| v.as_str()).map(ToString::to_string));
                let limit = crate::routes_usage_memory_kg::opencode_model_context_limit(
                    None,
                    m.as_deref().unwrap_or(""),
                );
                model = m;
                ctx = Some(md_hive::hooks::ContextSnapshot {
                    agent_id: id,
                    tokens: tok,
                    limit,
                    ts: now_ms(),
                });
            }
        }
    }
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
            let sleeping = info.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false);
            let pty_running = ptys.iter().any(|p| p.id == format!("pty-{id}"));
            let stopped = !pty_running && !sleeping;
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
            args.push("--standalone".to_string());
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

fn finish_worker_record(state: &AppState, w: WorkerRecord, result: &str, kill_pty: bool) {
    let tokens = worker_tokens_used(state, &w.worker_id, &w.cwd);
    let final_resp = extract_final_response(state, &w.pty_id);
    if kill_pty {
        let _ = state.pty.kill(&w.pty_id);
    }
    state.asked_compact.lock().unwrap().forget(&w.worker_id);
    if let Some(ref wt_path) = w.worktree {
        let p = std::path::Path::new(wt_path);
        if p.is_dir() {
            if md_git::is_repo_sync(p) {
                let _ = md_git::commit_all_sync(p, &format!("Temp {} ({result})", w.worker_id));
                if let Some(main_repo) = md_git::main_repo_sync(p)
                    && main_repo != p
                {
                    md_git::remove_worktree_sync(&main_repo, p);
                }
            }
            if p.exists()
                && state
                    .harness_home()
                    .is_some_and(|h| p.starts_with(h.join("worktrees")))
            {
                let _ = std::fs::remove_dir_all(p);
            }
        }
    }
    let now = now_ms();
    {
        let mut history = state.workers_history.lock().unwrap();
        history.push(worker_history_row(&w, result, now, tokens, final_resp));
        if history.len() > 200 {
            let drain_n = history.len() - 200;
            history.drain(..drain_n);
        }
        persist_workers_history(state, &history);
    }
    state.sync_hive_home();
    let unread_notice_lines: Vec<String> = {
        let mut hive = state.hive.lock().unwrap();
        let mut extra_unread = Vec::new();
        if result == "done" {
            let inbox_items = hive.inbox(&w.worker_id);
            let ids: Vec<String> = inbox_items
                .iter()
                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .collect();
            if !ids.is_empty() {
                for m in &inbox_items {
                    let subj = m.get("subject").and_then(|v| v.as_str()).unwrap_or("");
                    let act = m.get("act").and_then(|v| v.as_str()).unwrap_or("message");
                    let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let mid = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if subj != "Temp objective" && from != "temps" {
                        extra_unread.push(format!("- {act} {mid} from {from}: \"{subj}\""));
                    }
                }
                let count = ids.len();
                let _ = hive.file_inbox_messages(&w.worker_id, &ids);
                hive.append_log(&serde_json::json!({
                    "kind": "inbox-settled",
                    "agentId": w.worker_id,
                    "count": count,
                }));
            }
        }
        let _ = hive.patch_agent_field(&w.worker_id, "sleeping", Some(Value::Bool(false)));
        let _ = hive.patch_agent_field(&w.worker_id, "stopped", Some(Value::Bool(true)));
        let _ = hive.patch_agent_field(&w.worker_id, "archived", Some(Value::Bool(true)));
        let _ = hive.patch_agent_field(
            &w.worker_id,
            "status",
            Some(Value::String(result.to_string())),
        );
        hive.append_log(&serde_json::json!({
            "kind": "archive",
            "agentId": w.worker_id,
            "archived": true,
        }));
        extra_unread
    };
    if !unread_notice_lines.is_empty() {
        let recipient = if w.requested_by.is_empty() {
            "god"
        } else {
            w.requested_by.as_str()
        };
        let body = format!(
            "Worker {} signaled done and was released. These messages were still unread in its inbox and were filed under inbox/.done without an answer:\n{}\nIf one of them is not the order it just completed, nobody is working on it: resend it to another agent or re-hire the worker.",
            w.worker_id,
            unread_notice_lines.join("\n")
        );
        let subj = format!("[worker released — mail filed unread] {}", w.worker_id);
        let sent_ok = state
            .hive
            .lock()
            .unwrap()
            .send_message(
                serde_json::json!({
                    "to": recipient,
                    "act": "inform",
                    "subject": subj,
                    "body": body,
                }),
                "temps",
            )
            .is_ok();
        if sent_ok {
            deliver_or_enqueue_inner(
                state,
                recipient,
                format!("[HIVE inform from temps] {subj}\n{body}"),
                None,
                true,
            );
        }
    }
    state.hooks.forget_agent(&w.worker_id);
    state.wake_filing.lock().unwrap().forget(&w.worker_id);
    state.switch_clear.lock().unwrap().forget(&w.worker_id);
    state.codex_size.lock().unwrap().forget(&w.worker_id);
    state.asked_compact.lock().unwrap().forget(&w.worker_id);
    state.ended_agents.lock().unwrap().remove(&w.worker_id);
    state.pty.forget_pty(&w.pty_id);
    state.submitter.forget_pty(&w.pty_id);
    state.emit("hive:agentArchived", serde_json::json!({ "id": w.worker_id }));
    state.emit(
        "projects:changed",
        crate::routes_fs_git::build_projects_payload(state),
    );
}

pub(crate) fn prune_exited_workers(state: &AppState) {
    let ptys = state.pty.list();
    let exited_records = {
        let mut workers = state.workers.lock().unwrap();
        let mut exited = Vec::new();
        let mut i = 0;
        while i < workers.len() {
            if ptys.iter().any(|p| p.id == workers[i].pty_id) {
                i += 1;
            } else {
                exited.push(workers.remove(i));
            }
        }
        exited
    };
    if !exited_records.is_empty() {
        for w in exited_records {
            finish_worker_record(state, w, "exited", false);
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
    finish_worker_record(&state, w, "stopped", true);
    crate::hive_bootstrap::sync_fleet_json(&state);
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
            if objective.is_empty() {
                fail_file("missing required 'objective'");
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

            let has_registered_repos = {
                let mut cfg_store = state.config.lock().unwrap();
                !cfg_store.read().registered_repos.is_empty()
            };
            let clone_of = req
                .get("cloneOf")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let clone_meta = clone_of.and_then(|cid| {
                let reg = state.hive.lock().unwrap().registry();
                let obj = reg.get("agents").and_then(|a| a.get(cid))?;
                let src_name = obj
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(cid)
                    .to_string();
                let src_provider = obj
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let src_model = obj
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let src_cwd = obj
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(expand_tilde)
                    .filter(|p| p.is_dir());
                Some((cid.to_string(), src_name, src_provider, src_model, src_cwd))
            });
            let clone_cwd = clone_meta.as_ref().and_then(|(_, _, _, _, c)| c.clone());
            let mut clone_folder: Option<PathBuf> = None;
            if let Some(ref ccwd) = clone_cwd
                && !md_git::is_repo_sync(ccwd)
            {
                let wt_dir = home.join("worktrees").join(project_folder_name(&worker_id));
                if std::fs::create_dir_all(&wt_dir).is_ok() {
                    clone_folder = Some(wt_dir);
                }
            }

            let cwd_raw = req
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            let base_cwd = if let Some(ref cf) = clone_folder {
                cf.clone()
            } else if let Some(ref ccwd) = clone_cwd {
                ccwd.clone()
            } else if !cwd_raw.is_empty() {
                expand_tilde(cwd_raw)
            } else if !has_registered_repos {
                projects_root.join(project_folder_name(&worker_id))
            } else {
                fail_file("missing required 'cwd'");
                continue;
            };
            if in_hive_projects(&hive_root, &base_cwd) {
                if !base_cwd.exists() && std::fs::create_dir_all(&base_cwd).is_err() {
                    fail_file(&format!("could not create project folder: {}", base_cwd.display()));
                    continue;
                }
                if let Some(top) = hive_project_top_of(&hive_root, &base_cwd) {
                    let _ = md_git::init_repo_sync(&top, Some("Project folder made by Munder Difflin"));
                }
            }
            if !base_cwd.is_dir() {
                fail_file(&format!("cwd does not exist: {}", base_cwd.display()));
                continue;
            }

            let isolate = clone_folder.is_none()
                && (clone_cwd.is_some() || req.get("isolate").and_then(|v| v.as_bool()) != Some(false));
            let explicit_wt = req
                .get("worktree")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let mut created_worktree: Option<PathBuf> = None;
            let cwd_path = if isolate && explicit_wt.is_none() && md_git::is_repo_sync(&base_cwd) {
                let wt_dir = home.join("worktrees").join(project_folder_name(&worker_id));
                let branch = format!("temp/{worker_id}");
                let base_br = req.get("baseBranch").and_then(|v| v.as_str());
                if !wt_dir.exists()
                    && md_git::add_worktree_sync(&base_cwd, &wt_dir, &branch, base_br).is_ok()
                {
                    created_worktree = Some(wt_dir.clone());
                    wt_dir
                } else {
                    base_cwd.clone()
                }
            } else {
                base_cwd.clone()
            };
            let worktree_val = explicit_wt.or_else(|| {
                created_worktree
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            });
            let provider = req
                .get("provider")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| clone_meta.as_ref().and_then(|(_, _, p, _, _)| p.clone()))
                .unwrap_or_else(|| default_provider.clone());
            let model_opt = req
                .get("model")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| clone_meta.as_ref().and_then(|(_, _, _, m, _)| m.as_deref()))
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
                .map(ToString::to_string)
                .or_else(|| clone_meta.as_ref().map(|(_, sname, _, _, _)| format!("{sname} (temp)")))
                .unwrap_or_else(|| worker_id.clone());

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
                Some("temp"),
            );
            let spawn_res = state.pty.spawn(spawn_opts);
            if !spawn_res.ok {
                if let Some(ref wt) = created_worktree {
                    md_git::remove_worktree_sync(&base_cwd, wt);
                }
                if let Some(ref cf) = clone_folder {
                    let _ = std::fs::remove_dir_all(cf);
                }
                fail_file(spawn_res.error.as_deref().unwrap_or("spawn failed"));
                continue;
            }

            let done_dir = dir.join(".done");
            let _ = std::fs::create_dir_all(&done_dir);
            let _ = std::fs::rename(&path, done_dir.join(&fname));

            let mut entry = Map::new();
            entry.insert("id".to_string(), Value::String(worker_id.clone()));
            entry.insert("name".to_string(), Value::String(name.clone()));
            entry.insert("role".to_string(), Value::String("temp".to_string()));
            entry.insert("provider".to_string(), Value::String(provider.clone()));
            entry.insert("cwd".to_string(), Value::String(cwd_path.to_string_lossy().into_owned()));
            entry.insert("isTemp".to_string(), Value::Bool(true));
            entry.insert("archived".to_string(), Value::Bool(false));
            entry.insert("sleeping".to_string(), Value::Bool(false));
            entry.insert("stopped".to_string(), Value::Bool(false));
            entry.insert("status".to_string(), Value::String("working".to_string()));
            entry.insert("startedAt".to_string(), Value::Number(now_ms().into()));

            let hr_str = hive_root.to_string_lossy().into_owned();
            let cwd_disp = cwd_path.to_string_lossy().into_owned();
            let clone_line = clone_meta
                .as_ref()
                .map(|(cid, sname, _, _, src_cwd)| {
                    let mem = hive_root
                        .join("agents")
                        .join(cid)
                        .join("memory.md")
                        .to_string_lossy()
                        .into_owned();
                    let ro = if clone_folder.is_some() {
                        src_cwd
                            .as_ref()
                            .map(|p| format!(" Read source repo `{}` without editing it.", p.display()))
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    format!("[CLONE OF {sname} (`{cid}`)] Read source memory at `{mem}`.{ro}\n")
                })
                .unwrap_or_default();
            let memory_ctx_line = format!(
                "[CONTEXT] Before you start, read `{hr_str}/memory-index.md` and read the `memory.md` of any sibling whose role touches your task (read-only; write only your own).\n"
            );
            let full_objective = format!("{clone_line}{memory_ctx_line}{objective}");

            {
                let mut hive = state.hive.lock().unwrap();
                let _ = hive.register_worker(&worker_id, entry);
                let _ = hive.send_message(
                    serde_json::json!({
                        "to": worker_id,
                        "act": "request",
                        "subject": "Temp objective",
                        "body": full_objective,
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
                worktree: worktree_val,
                effort: req
                    .get("effort")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                requested_by: requester.clone(),
            });

            crate::hive_bootstrap::sync_fleet_json(state);
            state.emit(
                "projects:changed",
                crate::routes_fs_git::build_projects_payload(state),
            );
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
                    "role": "temp",
                }),
            );
            let job_prompt = format!(
                "{clone_line}{memory_ctx_line}[TEMP JOB from {requester}] {objective}\nYour workspace: `{cwd_disp}`\nYour AGENT_ID is `{worker_id}`. Read `{hr_str}/COMMANDS.md` if needed.\nWhen finished, write ONE JSON file to `{hr_str}/agents/{worker_id}/outbox/done-{worker_id}.json` containing:\n{{\"to\":\"{requester}\",\"act\":\"done\",\"subject\":\"Result\",\"body\":\"<concise summary of your findings/files written>\"}}"
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

pub(crate) fn run_compact_and_clear_requests(state: &AppState) {
    let Some(home) = state.harness_home() else {
        return;
    };
    let hive_root = home.join("hive");
    let now = now_ms();
    let reg = state.hive.lock().unwrap().registry();
    let agents_map = reg.get("agents").and_then(|v| v.as_object());
    let (compact_focus, ct_clear_rule) = {
        let mut store = state.config.lock().unwrap();
        let cfg = store.read();
        let ct = crate::routes_catalog::merged_context_trigger(cfg.extra.get("contextTrigger"));
        let focus = ct
            .get("compact")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let clear_rule = ct
            .get("clear")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({ "enabled": true }));
        (focus, clear_rule)
    };

    for (dir_name, is_clear) in [("compact-requests", false), ("clear-requests", true)] {
        let dir = hive_root.join(dir_name);
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
            let fname = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "req.json".to_string());
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let fail = |err: &str| {
                let failed = dir.join(".failed");
                let _ = std::fs::create_dir_all(&failed);
                let _ = std::fs::write(
                    failed.join(&fname),
                    serde_json::json!({ "error": err, "raw": raw }).to_string(),
                );
                let _ = std::fs::remove_file(&path);
            };
            let Ok(v) = serde_json::from_str::<Value>(&raw) else {
                fail("invalid JSON");
                continue;
            };
            let agent_id = v
                .get("agent")
                .or_else(|| v.get("agentId"))
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| fname.trim_end_matches(".json"));
            let agent_info = agents_map.and_then(|m| m.get(agent_id));
            let known = agent_info.is_some_and(|a| {
                !a.get("archived").and_then(|x| x.as_bool()).unwrap_or(false)
            });
            let sleeping = agent_info
                .and_then(|a| a.get("sleeping"))
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let provider = agent_info
                .and_then(|a| a.get("provider"))
                .and_then(|x| x.as_str())
                .unwrap_or("claude");
            let pty_id = format!("pty-{agent_id}");
            let live = state.pty.info(&pty_id).is_some();

            if !is_clear {
                if sleeping {
                    fail("agent is sleeping; nothing to compact");
                    continue;
                }
                if !live
                    && known
                    && let Some(info) = agent_info.and_then(|v| v.as_object())
                    && !info.get("isTemp").and_then(|v| v.as_bool()).unwrap_or(false)
                    && wake_agent_pty(state, agent_id, info)
                {
                    continue;
                }
                let can_compact = compaction_command_for_provider(provider, &compact_focus).is_some();
                if let Some(reason) = state
                    .asked_compact
                    .lock()
                    .unwrap()
                    .refusal(agent_id, known, live, can_compact, now)
                {
                    fail(reason);
                    continue;
                }
                state.asked_compact.lock().unwrap().ask(agent_id, now);
            } else if !live {
                if known
                    && let Some(info) = agent_info.and_then(|v| v.as_object())
                    && !info.get("isTemp").and_then(|v| v.as_bool()).unwrap_or(false)
                    && wake_agent_pty(state, agent_id, info)
                {
                    continue;
                }
                fail("agent PTY is not live");
                continue;
            }

            let done = dir.join(".done");
            let _ = std::fs::create_dir_all(&done);
            let _ = std::fs::rename(&path, done.join(&fname));

            state.hive.lock().unwrap().append_log(&serde_json::json!({
                "kind": if is_clear { "clear-requested" } else { "compact-requested" },
                "agentId": agent_id,
                "reason": v.get("reason").and_then(|r| r.as_str()).unwrap_or(""),
            }));
            if is_clear {
                state.emit(
                    "context:trigger",
                    serde_json::json!({ "action": "clear", "rule": ct_clear_rule }),
                );
                if state.events_tx.receiver_count() == 0
                    && let Some(cmd) = clear_command_for_provider(provider)
                {
                    deliver_or_enqueue(state, agent_id, cmd.to_string(), Some(provider));
                }
            }
        }
    }

    let waiting = state.asked_compact.lock().unwrap().waiting();
    for agent_id in waiting {
        state.emit(
            "hive:compactAsked",
            serde_json::json!({ "agentId": agent_id }),
        );
        if state.events_tx.receiver_count() == 0 {
            let provider = agents_map
                .and_then(|m| m.get(&agent_id))
                .and_then(|a| a.get("provider"))
                .and_then(|x| x.as_str())
                .unwrap_or("claude");
            if let Some(cmd) = compaction_command_for_provider(provider, &compact_focus) {
                deliver_or_enqueue(state, &agent_id, cmd, Some(provider));
            }
        }
    }
}

fn run_scheduled_missions(state: &AppState) {
    let now = now_ms();
    let (missions, ctx_trigger) = {
        let mut cfg_store = state.config.lock().unwrap();
        let cfg = cfg_store.read();
        let list = cfg
            .extra
            .get("missions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let ct = crate::routes_catalog::merged_context_trigger(cfg.extra.get("contextTrigger"));
        (list, ct)
    };
    if missions.is_empty() {
        return;
    }

    let mut updated_missions = missions.clone();
    let mut fired_any = false;

    for (idx, m) in missions.iter().enumerate() {
        if !m.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let interval_ms = m
            .get("intervalMs")
            .and_then(|v| v.as_u64())
            .filter(|&n| n > 0)
            .or_else(|| {
                m.get("intervalMinutes")
                    .and_then(|v| v.as_u64())
                    .filter(|&n| n > 0)
                    .map(|mins| mins * 60_000)
            })
            .unwrap_or(0);
        if interval_ms == 0 {
            continue;
        }
        let last_fired = m.get("lastFiredAt").and_then(|v| v.as_u64()).unwrap_or(0);
        if now.saturating_sub(last_fired) < interval_ms {
            continue;
        }

        fired_any = true;
        if let Some(obj) = updated_missions.get_mut(idx).and_then(|v| v.as_object_mut()) {
            obj.insert("lastFiredAt".to_string(), serde_json::json!(now));
        }

        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("mission");
        let kind = m.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let auto_compact = m.get("autoCompact").and_then(|v| v.as_bool()).unwrap_or(false);
        let label = m.get("label").and_then(|v| v.as_str()).unwrap_or(id);
        let body = m.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let to = m
            .get("to")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("god");

        if kind != "compact" {
            state.sync_hive_home();
            if (to == "temp" || to.starts_with("temp:"))
                && let Some(home) = state.harness_home()
            {
                let sr_dir = home.join("hive").join("spawn-requests");
                let _ = std::fs::create_dir_all(&sr_dir);
                let slug = format!("mission-{id}-{now}");
                let cwd = home.join("hive").join("projects").join(&slug);
                let doc = serde_json::json!({
                    "name": label,
                    "objective": body,
                    "cwd": cwd.to_string_lossy().into_owned(),
                    "cloneOf": to.strip_prefix("temp:").filter(|s| !s.is_empty()),
                });
                let _ = std::fs::write(sr_dir.join(format!("{slug}.json")), doc.to_string());
            } else {
                let target = if to == "temp" || to.starts_with("temp:") { "god" } else { to };
                let msg = serde_json::json!({
                    "to": target,
                    "act": "request",
                    "subject": label,
                    "body": body,
                });
                if let Ok(saved) = state.hive.lock().unwrap().send_message(msg, "scheduler") {
                    state.emit("hive:message", saved);
                    let text = format!("[SCHEDULED MISSION: {label}]\n{body}");
                    deliver_or_enqueue_inner(state, target, text, None, true);
                }
            }
        }

        if kind == "compact" || auto_compact {
            let rule = ctx_trigger
                .get("compact")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({ "enabled": true }));
            state.emit(
                "context:trigger",
                serde_json::json!({ "action": "compact", "rule": rule }),
            );
        }
    }

    if fired_any
        && let Ok(next_cfg) = state
            .config
            .lock()
            .unwrap()
            .update(serde_json::json!({ "missions": updated_missions }))
    {
        state.emit("missions:updated", serde_json::json!({ "ok": true }));
        if let Ok(val) = serde_json::to_value(&next_cfg) {
            state.emit("config:changed", val);
        }
    }
}

fn run_context_trigger_tick(
    state: &AppState,
    last_cadence_run: &mut HashMap<&'static str, u64>,
    last_pressure_compact: &mut HashMap<String, u64>,
) {
    let now = now_ms();
    let cfg = state.config.lock().unwrap().read();
    let ct = crate::routes_catalog::merged_context_trigger(cfg.extra.get("contextTrigger"));

    for action in ["compact", "clear"] {
        let Some(rule) = ct.get(action) else {
            continue;
        };
        let enabled = rule.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let every_ms = rule.get("everyMs").and_then(|v| v.as_u64()).unwrap_or(0);
        if !enabled || every_ms == 0 {
            continue;
        }
        let prev = *last_cadence_run.entry(action).or_insert(now);
        if now.saturating_sub(prev) >= every_ms {
            last_cadence_run.insert(action, now);
            state.emit(
                "context:trigger",
                serde_json::json!({ "action": action, "rule": rule }),
            );
        }
    }

    // Immediate token-pressure check when `compact.enabled` is true.
    if let Some(compact_rule) = ct.get("compact")
        && compact_rule
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    {
        let pct_std = compact_rule
            .get("minContextPct")
            .and_then(|v| v.as_u64())
            .unwrap_or(50);
        let pct_large = compact_rule
            .get("minContextPctLargeWindow")
            .and_then(|v| v.as_u64())
            .unwrap_or(30);
        let focus_msg = compact_rule
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let reg = state.hive.lock().unwrap().registry();
        let agents_map = reg.get("agents").and_then(|v| v.as_object());
        let contexts = state.hooks.context_by_id.lock().unwrap().clone();
        for (aid, snap) in contexts {
            if snap.limit == 0 || snap.tokens == 0 {
                continue;
            }
            let provider = agents_map
                .and_then(|m| m.get(&aid))
                .and_then(|a| a.get("provider"))
                .and_then(|v| v.as_str())
                .unwrap_or("claude");
            let Some(cmd) = compaction_command_for_provider(provider, focus_msg) else {
                continue;
            };
            let bar_pct = if snap.limit >= 600_000 { pct_large } else { pct_std };
            if bar_pct > 0 && snap.tokens.saturating_mul(100) >= snap.limit.saturating_mul(bar_pct) {
                let prev_a = last_pressure_compact.get(&aid).copied().unwrap_or(0);
                if now.saturating_sub(prev_a) >= 120_000 {
                    last_pressure_compact.insert(aid.clone(), now);
                    state.emit(
                        "hive:compactAsked",
                        serde_json::json!({ "agentId": aid }),
                    );
                    if state.events_tx.receiver_count() == 0 {
                        deliver_or_enqueue(state, &aid, cmd, Some(provider));
                    }
                }
            }
        }
    }
}

pub(crate) struct AppSubmitHooks<'a>(pub &'a AppState);

impl md_pty::submitter::SubmitHooks for AppSubmitHooks<'_> {
    fn prompt_seen(&self, agent_id: &str, since_ms: u64, tail: &str) -> bool {
        self.0
            .hooks
            .proof
            .lock()
            .unwrap()
            .prompt_seen(agent_id, since_ms, tail)
    }

    fn activity_seen(&self, agent_id: &str, since_ms: u64) -> bool {
        self.0
            .hooks
            .proof
            .lock()
            .unwrap()
            .activity_seen(agent_id, since_ms)
    }

    fn event_seen(&self, agent_id: &str, since_ms: u64, events: &[&str]) -> bool {
        self.0
            .hooks
            .proof
            .lock()
            .unwrap()
            .event_seen(agent_id, since_ms, events)
    }

    fn ask_open(&self, agent_id: &str) -> bool {
        self.0.hooks.proof.lock().unwrap().ask_open(agent_id)
    }

    fn compact_running_since(
        &self,
        agent_id: &str,
        since_ms: u64,
        max_ms: u64,
        now_ms: u64,
    ) -> u64 {
        self.0
            .hooks
            .proof
            .lock()
            .unwrap()
            .compact_running_since(agent_id, since_ms, max_ms, now_ms)
    }

    fn turn_open(&self, agent_id: &str, now_ms: u64) -> bool {
        self.0
            .hooks
            .proof
            .lock()
            .unwrap()
            .turn_open(agent_id, now_ms, 180_000)
    }
}

pub(crate) struct AppScreenProbe<'a> {
    state: &'a AppState,
    timed_out: std::sync::atomic::AtomicBool,
}

impl<'a> AppScreenProbe<'a> {
    pub(crate) fn new(state: &'a AppState) -> Self {
        Self {
            state,
            timed_out: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl md_pty::submitter::ScreenProbe for AppScreenProbe<'_> {
    fn probe<'a>(
        &'a self,
        pty_id: &'a str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<md_pty::submitter::TerminalProbeReply>> + Send + 'a>,
    > {
        Box::pin(async move {
            if self.state.events_tx.receiver_count() == 0
                || self.timed_out.load(std::sync::atomic::Ordering::Relaxed)
            {
                return None;
            }
            let info = self.state.pty.info(pty_id)?;
            let (probe_req, rx) = self.state.probes.register(pty_id, info.cols, info.rows);
            let probe_id = probe_req.id.clone();
            if let Ok(val) = serde_json::to_value(&probe_req) {
                self.state.emit("terminal:probe", val);
            }
            let reply = self.state.probes.wait_with_timeout(&probe_id, rx).await;
            if reply.is_none() {
                self.timed_out
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            reply
        })
    }
}

pub(crate) async fn probe_terminal_safe(state: &AppState, pty_id: &str) -> bool {
    if state.events_tx.receiver_count() == 0 {
        return true;
    }
    let Some(info) = state.pty.info(pty_id) else {
        return true;
    };
    let (probe_req, rx) = state.probes.register(pty_id, info.cols, info.rows);
    let probe_id = probe_req.id.clone();
    if let Ok(val) = serde_json::to_value(&probe_req) {
        state.emit("terminal:probe", val);
    }
    state
        .probes
        .wait_with_timeout(&probe_id, rx)
        .await
        .map(|reply| reply.safe)
        .unwrap_or(true)
}

fn run_wake_filing_quiet_tick(state: &AppState) {
    let waiting = state.wake_filing.lock().unwrap().waiting();
    if waiting.is_empty() {
        return;
    }
    let ptys = state.pty.list();
    let now = now_ms();
    let mut to_file: Vec<(String, Vec<String>)> = Vec::new();
    {
        let mut wf = state.wake_filing.lock().unwrap();
        for agent_id in waiting {
            let pty_id = format!("pty-{agent_id}");
            let last_out = ptys
                .iter()
                .find(|p| p.id == pty_id)
                .map(|p| p.last_output_at)
                .unwrap_or(0);
            let ids = wf.note_quiet(&agent_id, last_out, now);
            if !ids.is_empty() {
                to_file.push((agent_id, ids));
            }
        }
    }
    if !to_file.is_empty() {
        state.sync_hive_home();
        let hive = state.hive.lock().unwrap();
        for (agent_id, ids) in to_file {
            let _ = hive.file_inbox_messages(&agent_id, &ids);
        }
    }
}

const WORKER_WAKE_STALL_MS: u64 = 180_000;

fn handoff_path_for(state: &AppState, agent_id: &str) -> String {
    state
        .harness_home()
        .map(|h| {
            h.join("hive")
                .join("agents")
                .join(agent_id)
                .join("handoff.md")
                .to_string_lossy()
                .into_owned()
        })
        .unwrap_or_else(|| format!("hive/agents/{agent_id}/handoff.md"))
}

fn last_session_kept(state: &AppState, agent_id: &str) -> bool {
    state.harness_home().is_some_and(|h| {
        h.join("hive")
            .join("agents")
            .join(agent_id)
            .join("last-session.md")
            .is_file()
    })
}

fn switch_facts_for(state: &AppState, provider: &str, mail: &[Value]) -> WakeFacts {
    let ct = {
        let cfg = state.config.lock().unwrap().read();
        crate::routes_catalog::merged_context_trigger(cfg.extra.get("contextTrigger"))
    };
    WakeFacts {
        rule: SwitchClearRule::from_json(ct.get("switchClear")),
        wake_work: oldest_waking_work(mail),
        can_clear: switch_clear_command(provider).is_some(),
    }
}

fn inbox_wake_for(
    state: &AppState,
    agent_id: &str,
    provider: &str,
    inbox: &[Value],
) -> Option<String> {
    let pending = state.wake_filing.lock().unwrap().pending(agent_id, inbox);
    if pending.is_empty() {
        return None;
    }
    let inbox_path = state.harness_home().map(|h| {
        h.join("hive")
            .join("agents")
            .join(agent_id)
            .join("inbox")
            .to_string_lossy()
            .into_owned()
    });
    let (one_line, self_file) = md_hive::wake_filing::wake_options_for_provider(provider);
    let (wake_text, _) = md_hive::wake_filing::build_inbox_wake(
        &pending,
        inbox_path.as_deref(),
        one_line,
        self_file,
    );
    let handoff_path = handoff_path_for(state, agent_id);
    let facts = switch_facts_for(state, provider, &pending);
    let step = state
        .switch_clear
        .lock()
        .unwrap()
        .wake_step(agent_id, &facts, now_ms());
    match step {
        WakeStep::Handoff { work } => Some(handoff_request_text(work.as_deref(), &handoff_path)),
        WakeStep::Send { cleared: true } => Some(with_cleared_lead(
            &wake_text,
            &handoff_path,
            one_line,
            last_session_kept(state, agent_id),
        )),
        WakeStep::Send { cleared: false } => Some(wake_text),
    }
}

fn run_codex_size_beat(state: &AppState) {
    if state.harness_home().is_none() {
        return;
    }
    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    let agents_map = reg.get("agents").and_then(|v| v.as_object());
    let ptys = state.pty.list();
    let now = now_ms();
    for pty in ptys {
        if !pty.id.starts_with("pty-") {
            continue;
        }
        let agent_id = pty.id.strip_prefix("pty-").unwrap_or(&pty.id);
        let provider = agents_map
            .and_then(|m| m.get(agent_id))
            .and_then(|a| a.get("provider"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let is_codex = provider == "codex" || pty.command.to_lowercase().contains("codex");
        if !is_codex {
            continue;
        }
        let tp = state
            .hooks
            .transcript_paths
            .lock()
            .unwrap()
            .get(agent_id)
            .cloned();
        let due = state.codex_size.lock().unwrap().check(
            agent_id,
            tp.as_deref(),
            true,
            |path| std::fs::metadata(path).ok().map(|m| m.len()),
            now,
        );
        if due {
            state.emit(
                "hive:codexSizeClear",
                serde_json::json!({ "agentId": agent_id }),
            );
        }
    }
}

fn run_worker_wake_tick(state: &AppState, last_wake_by_agent: &mut HashMap<String, u64>) {
    const BOOT_GRACE_MS: u64 = 35_000;
    const QUIESCE_IDLE_MS: u64 = 12_000;
    const WAKE_COOLDOWN_MS: u64 = 60_000;

    state.sync_hive_home();
    let reg = state.hive.lock().unwrap().registry();
    let Some(agents) = reg.get("agents").and_then(|v| v.as_object()) else {
        return;
    };
    let ptys = state.pty.list();
    let now = now_ms();
    let waiting: std::collections::HashSet<String> =
        state.wake_filing.lock().unwrap().waiting().into_iter().collect();

    for (agent_id, info) in agents {
        if info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false)
            || info.get("onHold").and_then(|v| v.as_bool()).unwrap_or(false)
            || info.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false)
        {
            continue;
        }
        if waiting.contains(agent_id) {
            continue;
        }
        let prev_wake = last_wake_by_agent.get(agent_id).copied().unwrap_or(0);
        if now.saturating_sub(prev_wake) < WAKE_COOLDOWN_MS {
            continue;
        }
        let pty_id = format!("pty-{agent_id}");
        let Some(pty_info) = state.pty.info(&pty_id) else {
            continue;
        };
        if now.saturating_sub(pty_info.started_at) < BOOT_GRACE_MS {
            continue;
        }
        let last_output_at = ptys
            .iter()
            .find(|p| p.id == pty_id)
            .map(|p| p.last_output_at)
            .unwrap_or(0);
        if last_output_at == 0 || now.saturating_sub(last_output_at) < QUIESCE_IDLE_MS {
            continue;
        }
        {
            let ctl = state.control.lock().unwrap();
            let snap = ctl.snapshot(agent_id);
            if snap.paused || snap.halted || snap.auto_delivery_paused {
                continue;
            }
        }
        if state.hooks.proof.lock().unwrap().ask_open(agent_id) {
            continue;
        }

        let inbox = state.hive.lock().unwrap().inbox(agent_id);
        let pending = state.wake_filing.lock().unwrap().pending(agent_id, &inbox);
        if pending.is_empty() {
            continue;
        }
        let provider = info
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("claude")
            .to_string();
        let facts = switch_facts_for(state, &provider, &pending);
        let oldest_ms = pending
            .iter()
            .filter_map(|m| {
                m.get("created_at")
                    .and_then(|v| v.as_str())
                    .and_then(md_hive::switch_clear::parse_iso_ms)
            })
            .min()
            .unwrap_or(0);
        if state.switch_clear.lock().unwrap().left_to_window(
            agent_id,
            &facts,
            oldest_ms,
            WORKER_WAKE_STALL_MS,
            now,
        ) {
            continue;
        }
        let Some(wake_text) = inbox_wake_for(state, agent_id, &provider, &inbox) else {
            continue;
        };

        last_wake_by_agent.insert(agent_id.clone(), now);
        let st_clone = state.clone();
        let aid = agent_id.clone();
        let prov_done = provider.clone();
        tokio::spawn(async move {
            if !probe_terminal_safe(&st_clone, &pty_id).await {
                st_clone.switch_clear.lock().unwrap().abandon(&aid);
                return;
            }
            let st_began = st_clone.clone();
            let probe = AppScreenProbe::new(&st_clone);
            let hooks = AppSubmitHooks(&st_clone);
            if let Ok(res) = st_clone
                .submitter
                .submit_with_context(
                    md_pty::submitter::PtySubmitRequest {
                        pty_id,
                        agent_id: Some(aid.clone()),
                        provider,
                        text: wake_text.clone(),
                        source: "nudge".to_string(),
                        settle_ms: Some(250),
                        press_only: false,
                        idle: Some(true),
                        ref_id: None,
                    },
                    Some(&probe),
                    &hooks,
                    move |r| {
                        st_began.emit("pty:began", serde_json::json!(r.pty_id));
                    },
                )
                .await
            {
                note_agent_prompt_submitted(
                    &st_clone,
                    &aid,
                    &wake_text,
                    &res.outcome,
                    res.ms,
                    false,
                    Some(&prov_done),
                );
            }
        });
    }
}

pub(crate) fn process_outboxes_tick(state: &AppState) {
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
                    deliver_or_enqueue_inner(state, tid, text, None, true);
                }
            }
        }
        if from.starts_with("worker-") && act == "done" {
            let done_worker = {
                let mut workers = state.workers.lock().unwrap();
                workers
                    .iter()
                    .position(|w| w.worker_id == from)
                    .map(|idx| workers.remove(idx))
            };
            if let Some(w) = done_worker {
                finish_worker_record(state, w, "done", true);
                crate::hive_bootstrap::sync_fleet_json(state);
            }
        }
    }
}

pub fn spawn_hive_poller(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(750));
        let mut ticks: u64 = 0;
        let mut last_cadence_run: HashMap<&'static str, u64> = HashMap::new();
        let mut last_pressure_compact: HashMap<String, u64> = HashMap::new();
        let mut last_wake_by_agent: HashMap<String, u64> = HashMap::new();
        loop {
            interval.tick().await;
            ticks += 1;
            process_outboxes_tick(&state);
            process_spawn_requests(&state);
            process_hire_requests(&state);
            prune_exited_workers(&state);
            if ticks.is_multiple_of(4) {
                run_wake_filing_quiet_tick(&state);
                run_compact_and_clear_requests(&state);
                run_scheduled_missions(&state);
                run_context_trigger_tick(&state, &mut last_cadence_run, &mut last_pressure_compact);
            }
            if ticks == 1 || ticks.is_multiple_of(8) {
                run_worker_wake_tick(&state, &mut last_wake_by_agent);
                evaluate_circuit_breaker_tick(&state);
                crate::hive_bootstrap::sync_fleet_json(&state);
            }
            if ticks.is_multiple_of(20) {
                run_codex_size_beat(&state);
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

fn wake_agent_pty(state: &AppState, agent_id: &str, info: &Map<String, Value>) -> bool {
    let pty_id = format!("pty-{agent_id}");
    if state.pty.info(&pty_id).is_some() {
        return true;
    }
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
        .unwrap_or(agent_id)
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
    let is_god = info
        .get("isGod")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || agent_id == "god";
    let seed = crate::hive_bootstrap::prepare_agent_spawn(
        state,
        &mut spawn_opts,
        agent_id,
        &name,
        &provider,
        is_god,
        role,
    );
    let res = state.pty.spawn(spawn_opts);
    if !res.ok {
        return false;
    }
    state.ended_agents.lock().unwrap().remove(agent_id);
    state.emit(
        "hive:agentSpawned",
        serde_json::json!({
            "id": agent_id,
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
        let aid = agent_id.to_string();
        let prov = provider;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
            deliver_or_enqueue(&st_clone, &aid, s, Some(&prov));
        });
    }
    true
}

async fn hive_wake(State(state): State<AppState>, Json(req): Json<WakeReq>) -> Json<Value> {
    state.sync_hive_home();
    let pty_id = format!("pty-{}", req.agent_id);
    let was_live = state.pty.info(&pty_id).is_some();

    let (is_archived_or_ended, agent_info) = {
        let home = state.harness_home();
        let archived_in_roster = state
            .roster
            .lock()
            .unwrap()
            .read(home.as_deref())
            .map(|snap| {
                snap.archived
                    .iter()
                    .any(|a| a.get("id").and_then(|v| v.as_str()) == Some(req.agent_id.as_str()))
                    || snap.agents.iter().any(|a| {
                        a.get("id").and_then(|v| v.as_str()) == Some(req.agent_id.as_str())
                            && a.get("archived").and_then(|v| v.as_bool()).unwrap_or(false)
                    })
            })
            .unwrap_or(false);
        let ended = state.ended_agents.lock().unwrap().contains_key(&req.agent_id);

        let mut hive = state.hive.lock().unwrap();
        let reg = hive.registry();
        let reg_entry = reg
            .get("agents")
            .and_then(|a| a.get(&req.agent_id))
            .and_then(|v| v.as_object())
            .cloned();
        let archived_in_reg = reg_entry
            .as_ref()
            .and_then(|o| o.get("archived"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let is_god = req.agent_id == "god"
            || reg_entry
                .as_ref()
                .and_then(|o| o.get("isGod"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
        let blocked = !is_god && (archived_in_roster || archived_in_reg || ended);
        if !blocked {
            let _ = hive.patch_agent_field(&req.agent_id, "sleeping", Some(Value::Bool(false)));
            let _ = hive.patch_agent_field(&req.agent_id, "stopped", Some(Value::Bool(false)));
            let _ = hive.patch_agent_field(
                &req.agent_id,
                "status",
                Some(Value::String("idle".to_string())),
            );
        }
        (blocked, reg_entry)
    };

    if is_archived_or_ended {
        return Json(serde_json::json!({ "ok": false, "skipped": "archived" }));
    }

    if !was_live
        && let Some(ref info) = agent_info
    {
        wake_agent_pty(&state, &req.agent_id, info);
    }

    // When called by a connected UI (`enqueueMessage` calls `wakeAgent` unconditionally),
    // do NOT echo `hive:enqueue` back over WebSocket or it triggers an enqueue<->wake loop.
    // Headless mode (no WS listeners) still wakes unread inbox directly via submitter.
    if state.events_tx.receiver_count() == 0 {
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
    }
    crate::hive_bootstrap::sync_fleet_json(&state);
    Json(serde_json::json!({ "ok": true }))
}

async fn hive_wake_retry(State(state): State<AppState>, Json(req): Json<WakeReq>) -> Json<Value> {
    let ids = state.wake_filing.lock().unwrap().retry(&req.agent_id);
    if !ids.is_empty() {
        state.sync_hive_home();
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "wake-retry",
            "agentId": req.agent_id,
            "ids": ids,
        }));
    }
    let _ = hive_wake(State(state), Json(req)).await;
    Json(Value::Array(ids.into_iter().map(Value::String).collect()))
}

async fn hive_wake_text(Path(id): Path<String>, State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    let (inbox, provider) = {
        let mut hive = state.hive.lock().unwrap();
        let inbox = hive.inbox(&id);
        let provider = hive
            .registry()
            .get("agents")
            .and_then(|a| a.get(&id))
            .and_then(|info| info.get("provider"))
            .and_then(|v| v.as_str())
            .unwrap_or("claude")
            .to_string();
        (inbox, provider)
    };
    match inbox_wake_for(&state, &id, &provider, &inbox) {
        Some(text) => Json(Value::String(text)),
        None => Json(Value::Null),
    }
}

async fn hive_switch_clear_gate(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<Value> {
    let gate = state.switch_clear.lock().unwrap().clear_gate(&id, now_ms());
    let s = match gate {
        ClearGate::Go => "go",
        ClearGate::Hold => "hold",
        ClearGate::Drop => "drop",
    };
    Json(Value::String(s.to_string()))
}

async fn hive_codex_size_clear_text(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<Value> {
    state.sync_hive_home();
    let provider = {
        let mut hive = state.hive.lock().unwrap();
        hive.registry()
            .get("agents")
            .and_then(|a| a.get(&id))
            .and_then(|info| info.get("provider"))
            .and_then(|v| v.as_str())
            .unwrap_or("claude")
            .to_string()
    };
    if provider != "codex" {
        return Json(Value::Null);
    }
    let now = now_ms();
    let sc = state.switch_clear.lock().unwrap();
    let phase = sc.phase(&id);
    let open_bg = sc.background.open(&id, now);
    let on_work = sc.on_work(&id).map(ToString::to_string);
    drop(sc);
    let due = state.codex_size.lock().unwrap().handoff_due(
        &id,
        phase,
        open_bg,
        switch_clear_command(&provider).is_some(),
    );
    if !due {
        return Json(Value::Null);
    }
    let text = size_handoff_request_text(on_work.as_deref(), &handoff_path_for(&state, &id));
    Json(Value::String(text))
}

async fn hive_wake_unconfirmed(Path(id): Path<String>, State(state): State<AppState>) -> Json<Value> {
    state.sync_hive_home();
    let inbox = state.hive.lock().unwrap().inbox(&id);
    let ids = state.wake_filing.lock().unwrap().unconfirmed(&id, &inbox);
    Json(Value::Array(ids.into_iter().map(Value::String).collect()))
}

async fn hive_ended_agents(State(state): State<AppState>) -> Json<Value> {
    let ptys = state.pty.list();
    let live_ptys: std::collections::HashSet<&str> =
        ptys.iter().map(|p| p.id.as_str()).collect();
    let ended = state.ended_agents.lock().unwrap();
    let rows: Vec<Value> = ended
        .values()
        .filter(|e| !live_ptys.contains(format!("pty-{}", e.agent_id).as_str()))
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();
    Json(Value::Array(rows))
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HireDoneReq {
    req_id: String,
    answer: Option<Value>,
}

fn file_hire_result(dir: &std::path::Path, src: &std::path::Path, sub: &str, name: &str, body: &Value) {
    let target_dir = dir.join(sub);
    let _ = std::fs::create_dir_all(&target_dir);
    let _ = std::fs::write(
        target_dir.join(name),
        serde_json::to_string_pretty(body).unwrap_or_default(),
    );
    let _ = std::fs::remove_file(src);
}

fn inform_god_hire(state: &AppState, subject: &str, body: &str) {
    state.sync_hive_home();
    let msg = serde_json::json!({
        "to": "god",
        "act": "inform",
        "subject": subject,
        "body": body,
    });
    if let Ok(saved) = state.hive.lock().unwrap().send_message(msg, "app") {
        state.emit("hive:message", saved);
        let text = format!("[HIRE] {subject}\n{body}");
        deliver_or_enqueue_inner(state, "god", text, None, true);
    }
}

pub(crate) fn process_hire_requests(state: &AppState) {
    let Some(home) = state.harness_home() else {
        return;
    };
    let hive_root = home.join("hive");
    let dir = hive_root.join("hire-requests");
    if !dir.is_dir() {
        return;
    }

    let now = now_ms();
    // Clean expired or orphaned inflight requests
    let inflight_timed_out = {
        let mut guard = state.hire_inflight.lock().unwrap();
        if let Some(ref cur) = *guard {
            if now.saturating_sub(cur.started_at) >= 300_000 {
                guard.take()
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(cur) = inflight_timed_out {
        let raw_val: Value = std::fs::read_to_string(&cur.file_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(Value::Null);
        let err = "the app window did not add the agent in 5 minutes";
        file_hire_result(
            &dir,
            &cur.file_path,
            ".failed",
            &format!("{}.json", cur.req_id),
            &serde_json::json!({ "error": err, "request": raw_val }),
        );
        inform_god_hire(state, &format!("Hire request {} failed", cur.req_id), err);
    }

    if state.hire_inflight.lock().unwrap().is_none()
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        for entry in entries.flatten() {
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with(".inflight-") && fname.ends_with(".json") && p.is_file() {
                let req_id = fname
                    .trim_start_matches(".inflight-")
                    .trim_end_matches(".json")
                    .to_string();
                let raw_val: Value = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
                    .unwrap_or(Value::Null);
                file_hire_result(
                    &dir,
                    &p,
                    ".failed",
                    &format!("{req_id}.json"),
                    &serde_json::json!({
                        "error": "the app stopped while this agent was being added",
                        "request": raw_val
                    }),
                );
            }
        }
    }

    if state.hire_inflight.lock().unwrap().is_some() {
        return;
    }
    let (may_add, registered_repos) = {
        let mut cfg_store = state.config.lock().unwrap();
        let cfg = cfg_store.read();
        let may = cfg
            .extra
            .get("orchestratorMayAddLasting")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        (may, cfg.registered_repos)
    };
    if !may_add
        || !state.hire_runner_ready.load(std::sync::atomic::Ordering::Relaxed)
        || state.events_tx.receiver_count() == 0
    {
        return;
    }

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|s| s.to_str()) == Some("json")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    files.sort();
    let Some(file) = files.into_iter().next() else {
        return;
    };
    let fname = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "hire.json".to_string());
    let req_id = fname.trim_end_matches(".json").to_string();
    let raw_text = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => return,
    };
    let raw_val: Value = match serde_json::from_str(&raw_text) {
        Ok(v) => v,
        Err(_) => {
            file_hire_result(
                &dir,
                &file,
                ".failed",
                &fname,
                &serde_json::json!({ "error": "invalid JSON in hire request" }),
            );
            return;
        }
    };
    if dir.join(".done").join(&fname).is_file() {
        let err = "a request with this id was run before; give a new request a new file name";
        file_hire_result(
            &dir,
            &file,
            ".failed",
            &fname,
            &serde_json::json!({ "error": err, "request": raw_val }),
        );
        inform_god_hire(state, &format!("Hire request {req_id} rejected"), err);
        return;
    }
    let name = raw_val
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    if name.is_empty() {
        let err = "the file names no agent";
        file_hire_result(
            &dir,
            &file,
            ".failed",
            &fname,
            &serde_json::json!({ "error": err, "request": raw_val }),
        );
        inform_god_hire(state, &format!("Hire request {req_id} rejected"), err);
        return;
    }

    let reg = state.hive.lock().unwrap().registry();
    let base_id = project_folder_name(name);
    let mut agent_id = base_id.clone();
    let mut n = 2usize;
    while reg
        .get("agents")
        .and_then(|a| a.get(&agent_id))
        .is_some()
        || state.pty.info(&format!("pty-{agent_id}")).is_some()
    {
        agent_id = format!("{base_id}-{n}");
        n += 1;
    }

    let cfg_obj = raw_val.get("configs");
    let mut folder: Option<String> = None;
    if let Some(cwd_str) = raw_val.get("cwd").and_then(|v| v.as_str()).map(str::trim)
        && !cwd_str.is_empty()
    {
        let expanded = expand_tilde(cwd_str);
        let allowed_registered = registered_repos.iter().any(|r| expand_tilde(r) == expanded);
        let allowed_project = in_hive_projects(&hive_root, &expanded);
        if !allowed_registered && !allowed_project {
            let err = format!(
                "the folder {cwd_str} is not in registeredRepos nor under hive/projects"
            );
            file_hire_result(
                &dir,
                &file,
                ".failed",
                &fname,
                &serde_json::json!({ "error": err, "request": raw_val }),
            );
            inform_god_hire(state, &format!("Hire request {req_id} rejected"), &err);
            return;
        }
        if allowed_project {
            let _ = std::fs::create_dir_all(&expanded);
            if let Some(top) = hive_project_top_of(&hive_root, &expanded) {
                let _ = md_git::init_repo_sync(&top, Some("Project folder made by Munder Difflin"));
            }
        }
        folder = Some(expanded.to_string_lossy().into_owned());
    } else if registered_repos.is_empty() {
        let default_dir = hive_root.join("projects").join(&agent_id);
        let _ = std::fs::create_dir_all(&default_dir);
        let _ = md_git::init_repo_sync(&default_dir, Some("Project folder made by Munder Difflin"));
        folder = Some(default_dir.to_string_lossy().into_owned());
    }

    let claimed = dir.join(format!(".inflight-{fname}"));
    if std::fs::rename(&file, &claimed).is_err() {
        return;
    }

    let mut card = serde_json::Map::new();
    card.insert("name".to_string(), Value::String(name.to_string()));
    if let Some(c) = cfg_obj.and_then(|v| v.as_object()) {
        for key in ["role", "goal", "character", "accent", "provider", "model", "commandFlags", "capabilities"] {
            if let Some(val) = c.get(key)
                && !val.is_null()
            {
                card.insert(key.to_string(), val.clone());
            }
        }
        let isolate = c.get("isolate").and_then(|v| v.as_bool()).unwrap_or(true);
        card.insert("isolate".to_string(), Value::Bool(isolate));
    } else {
        card.insert("isolate".to_string(), Value::Bool(true));
    }

    *state.hire_inflight.lock().unwrap() = Some(crate::state::HireInflight {
        req_id: req_id.clone(),
        file_path: claimed,
        agent_id: agent_id.clone(),
        name: name.to_string(),
        started_at: now,
    });
    let mut job = serde_json::json!({
        "reqId": req_id,
        "agentId": agent_id,
        "card": Value::Object(card),
    });
    if let Some(cwd_val) = folder {
        job.as_object_mut()
            .unwrap()
            .insert("cwd".to_string(), Value::String(cwd_val));
    }
    state.emit("hire:request", job);
}

async fn hire_runner_ready(State(state): State<AppState>) -> Json<Value> {
    state
        .hire_runner_ready
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Json(serde_json::json!({ "ok": true }))
}

async fn hire_request_done(
    State(state): State<AppState>,
    Json(req): Json<HireDoneReq>,
) -> Json<Value> {
    let inflight = {
        let mut guard = state.hire_inflight.lock().unwrap();
        if guard.as_ref().is_some_and(|c| c.req_id == req.req_id) {
            guard.take()
        } else {
            None
        }
    };
    let Some(cur) = inflight else {
        return Json(serde_json::json!({ "ok": false, "error": "no matching inflight hire request" }));
    };
    let Some(home) = state.harness_home() else {
        return Json(serde_json::json!({ "ok": false, "error": "no harnessHome" }));
    };
    let dir = home.join("hive").join("hire-requests");
    let raw_val: Value = std::fs::read_to_string(&cur.file_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let ok = req
        .answer
        .as_ref()
        .and_then(|a| a.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if ok {
        file_hire_result(
            &dir,
            &cur.file_path,
            ".done",
            &format!("{}.json", cur.req_id),
            &serde_json::json!({
                "agentId": cur.agent_id,
                "name": cur.name,
                "request": raw_val,
            }),
        );
        state.emit(
            "projects:changed",
            crate::routes_fs_git::build_projects_payload(&state),
        );
        state.hive.lock().unwrap().append_log(&serde_json::json!({
            "kind": "hire-request-done",
            "reqId": cur.req_id,
            "agentId": cur.agent_id,
            "name": cur.name,
        }));
        inform_god_hire(
            &state,
            &format!("Hired {} ({})", cur.name, cur.agent_id),
            &format!("Agent `{}` (`{}`) has been added to the floor.", cur.name, cur.agent_id),
        );
    } else {
        let err = req
            .answer
            .as_ref()
            .and_then(|a| a.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or("the start was refused");
        file_hire_result(
            &dir,
            &cur.file_path,
            ".failed",
            &format!("{}.json", cur.req_id),
            &serde_json::json!({
                "error": err,
                "request": raw_val,
            }),
        );
        inform_god_hire(&state, &format!("Hire request {} failed", cur.req_id), err);
    }
    Json(serde_json::json!({ "ok": true }))
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
        .route("/api/v1/hive/wake-retry", post(hive_wake_retry))
        .route("/api/v1/hive/wake-text/{id}", get(hive_wake_text))
        .route("/api/v1/hive/wake-unconfirmed/{id}", get(hive_wake_unconfirmed))
        .route("/api/v1/hive/switch-clear-gate/{id}", get(hive_switch_clear_gate))
        .route("/api/v1/hive/codex-size-clear-text/{id}", get(hive_codex_size_clear_text))
        .route("/api/v1/hive/ended-agents", get(hive_ended_agents))
        .route("/api/v1/pty/screen-lines", post(pty_screen_lines))
        .route("/api/v1/workers", get(workers_list))
        .route("/api/v1/workers/history", get(workers_history))
        .route("/api/v1/workers/{id}/stop", post(worker_stop))
        .route("/api/v1/hive/hire-runner-ready", post(hire_runner_ready))
        .route("/api/v1/hive/hire-done", post(hire_request_done))
}
