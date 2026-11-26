//! HTTP routes for Phase 5: local transcript usage, memory manager, and Knowledge Graph.

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use base64::Engine;
use md_core::config::{default_data_dir, expand_tilde};
use md_hive::{
    kg::{self, KgIngestInput},
    memory,
    usage::{default_projects_root, read_agent_usage_in, read_context_tokens},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsageQuery {
    cwd: String,
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextQuery {
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenLimitReq {
    agent_id: String,
    limit: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenLimitsViewReq {
    #[serde(default)]
    agent_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemoryWriteReq {
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchQuery {
    q: String,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPageQuery {
    agent_id: Option<String>,
    limit: Option<usize>,
    after: Option<usize>,
    before: Option<usize>,
}

fn read_opencode_session_items(cwd: &str) -> Option<(String, Vec<Value>)> {
    let home = dirs::home_dir()?;
    let db_path = home.join(".local/share/opencode/opencode.db");
    if !db_path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let session_id: String = conn
        .query_row(
            "SELECT id FROM session WHERE directory = ?1 ORDER BY time_updated DESC LIMIT 1",
            [cwd],
            |r| r.get(0),
        )
        .ok()?;

    let mut msg_stmt = conn
        .prepare("SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created ASC, id ASC")
        .ok()?;
    let mut part_stmt = conn
        .prepare("SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created ASC, id ASC")
        .ok()?;

    let mut texts_by_msg: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let part_rows = part_stmt
        .query_map([&session_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .ok()?;
    for row in part_rows.flatten() {
        if let Ok(v) = serde_json::from_str::<Value>(&row.1)
            && v.get("type").and_then(|t| t.as_str()) == Some("text")
            && let Some(txt) = v.get("text").and_then(|t| t.as_str())
            && !txt.trim().is_empty()
        {
            texts_by_msg.entry(row.0).or_default().push(txt.to_string());
        }
    }

    let mut items = Vec::new();
    let msg_rows = msg_stmt
        .query_map([&session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .ok()?;
    for row in msg_rows.flatten() {
        let (msg_id, ts, raw_data) = row;
        let role = serde_json::from_str::<Value>(&raw_data)
            .ok()
            .and_then(|v| v.get("role").and_then(|r| r.as_str()).map(ToString::to_string))
            .unwrap_or_else(|| "assistant".to_string());
        let kind = if role == "user" { "user" } else { "assistant" };
        let Some(parts) = texts_by_msg.get(&msg_id) else {
            continue;
        };
        let text = parts.join("\n\n");
        if text.trim().is_empty() {
            continue;
        }
        items.push(json!({
            "id": msg_id,
            "kind": kind,
            "at": ts,
            "text": text,
            "truncated": false,
        }));
    }
    Some((session_id, items))
}

async fn history_page(
    State(state): State<AppState>,
    Query(q): Query<HistoryPageQuery>,
) -> Json<Value> {
    let agent_id = q.agent_id.as_deref().unwrap_or("");
    let limit = q.limit.unwrap_or(150).clamp(1, 500);
    let home = state.harness_home();

    let (cwd, provider) = {
        let mut found_cwd = None;
        let mut found_prov = None;
        if let Some(snap) = state.roster.lock().unwrap().read(home.as_deref()) {
            for a in snap.agents.iter().chain(snap.archived.iter()) {
                if a.get("id").and_then(|v| v.as_str()) == Some(agent_id) {
                    found_cwd = a.get("cwd").and_then(|v| v.as_str()).map(ToString::to_string);
                    found_prov = a.get("provider").and_then(|v| v.as_str()).map(ToString::to_string);
                    break;
                }
            }
        }
        (
            found_cwd.or_else(|| home.as_ref().map(|h| h.to_string_lossy().into_owned())),
            found_prov.unwrap_or_default(),
        )
    };

    let (file_id, prov_name, all_items) = if (provider == "opencode" || provider.is_empty())
        && let Some(ref c) = cwd
        && let Some((sid, items)) = read_opencode_session_items(c)
    {
        (sid, "opencode".to_string(), items)
    } else {
        let rows = state
            .db
            .lock()
            .unwrap()
            .list_history(
                if agent_id.is_empty() { None } else { Some(agent_id) },
                md_db::COMMAND_HISTORY_LIMIT,
            )
            .unwrap_or_default();
        let items: Vec<Value> = rows
            .into_iter()
            .rev()
            .map(|r| {
                json!({
                    "id": r.id.to_string(),
                    "kind": "user",
                    "at": r.ts,
                    "text": r.text,
                    "truncated": false,
                })
            })
            .collect();
        (format!("db-{agent_id}"), "history".to_string(), items)
    };

    let total = all_items.len();
    let (start, end, slice) = if let Some(after) = q.after {
        let s = after.min(total);
        (s, total, all_items[s..].to_vec())
    } else if let Some(before) = q.before {
        let e = before.min(total);
        let s = e.saturating_sub(limit);
        (s, e, all_items[s..e].to_vec())
    } else {
        let s = total.saturating_sub(limit);
        (s, total, all_items[s..].to_vec())
    };

    Json(json!({
        "ok": true,
        "fileId": file_id,
        "provider": prov_name,
        "fileName": file_id,
        "start": start,
        "end": end,
        "atStart": start == 0,
        "live": true,
        "items": slice,
        "hasMore": start > 0,
    }))
}

fn resolve_kg_root(state: &AppState) -> (PathBuf, bool) {
    let cfg = {
        let mut cfg_store = state.config.lock().unwrap();
        cfg_store.read()
    };
    let kg_obj = cfg.extra.get("knowledgeGraph").and_then(|v| v.as_object());
    let enabled = kg_obj
        .and_then(|o| o.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let root = kg_obj
        .and_then(|o| o.get("rootPath"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| {
            cfg.harness_home
                .as_deref()
                .map(|h| PathBuf::from(h).join("knowledge"))
                .unwrap_or_else(|| default_data_dir().join("knowledge"))
        });
    (root, enabled)
}

async fn usage_agent(Query(q): Query<UsageQuery>) -> Json<Value> {
    let root = default_projects_root();
    let res = read_agent_usage_in(&root, &q.cwd, q.session_id.as_deref());
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
}

pub(crate) fn collect_agent_usage_rows(
    state: &AppState,
) -> Vec<(String, md_hive::usage::AgentUsage)> {
    let home = state.harness_home();
    let root = default_projects_root();
    let snap = state.roster.lock().unwrap().read(home.as_deref());
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(s) = snap {
        for a in s.agents.iter().chain(s.archived.iter()) {
            let Some(id) = a.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let cwd = a
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(expand_tilde)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if cwd.is_empty() {
                continue;
            }
            let sid = a.get("sessionId").and_then(|v| v.as_str());
            let u = read_agent_usage_in(&root, &cwd, sid);
            rows.push((id.to_string(), u));
        }
    }
    rows
}

fn collect_usage_samples(state: &AppState) -> (Vec<Value>, HashMap<String, Value>) {
    let mut usage_rows = Vec::new();
    let mut usage_map = HashMap::new();
    for (id, u) in collect_agent_usage_rows(state) {
        let row = json!({
            "agentId": id,
            "input": u.input_tokens,
            "output": u.output_tokens,
            "cacheWrite": u.cache_write_tokens,
            "cacheRead": u.cache_read_tokens,
            "usd": u.estimated_cost_usd,
            "model": u.model,
            "ts": u.as_of,
        });
        usage_rows.push(row.clone());
        usage_map.insert(id, row);
    }
    (usage_rows, usage_map)
}

async fn telemetry_snapshot(State(state): State<AppState>) -> Json<Value> {
    let (usage, _) = collect_usage_samples(&state);
    Json(json!({
        "usage": usage,
        "spans": {},
    }))
}

async fn telemetry_usage(
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let (_, map) = collect_usage_samples(&state);
    if let Some(id) = q.get("agentId") {
        return Json(map.get(id).cloned().unwrap_or(Value::Null));
    }
    Json(serde_json::to_value(map).unwrap_or_else(|_| json!({})))
}

async fn telemetry_spans() -> Json<Value> {
    Json(json!([]))
}

async fn usage_context(Query(q): Query<ContextQuery>) -> Json<Value> {
    let tokens = read_context_tokens(std::path::Path::new(&q.path));
    Json(json!({ "tokens": tokens }))
}

async fn config_set_token_limit(
    State(state): State<AppState>,
    Json(req): Json<TokenLimitReq>,
) -> Json<Value> {
    let mut cfg_store = state.config.lock().unwrap();
    let current = cfg_store.read();
    let mut limits = current
        .extra
        .get("tokenLimits")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    match req.limit {
        Some(lim) if !lim.is_null() => {
            limits.insert(req.agent_id, lim);
        }
        _ => {
            limits.remove(&req.agent_id);
        }
    }
    match cfg_store.update(json!({ "tokenLimits": limits })) {
        Ok(next) => {
            if let Ok(val) = serde_json::to_value(&next) {
                state.emit("config:changed", val);
            }
            Json(json!({ "ok": true, "tokenLimits": next.extra.get("tokenLimits") }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

fn normalize_limit_entry(raw: Option<&Value>) -> Option<Value> {
    let v = raw?;
    if v.is_null() {
        return None;
    }
    if let Some(cap) = v.as_u64() {
        return Some(json!({ "period": "day", "cap": cap }));
    }
    if v.is_object() {
        return Some(v.clone());
    }
    None
}

async fn token_limits_view(
    State(state): State<AppState>,
    Json(req): Json<TokenLimitsViewReq>,
) -> Json<Value> {
    let cfg = state.config.lock().unwrap().read();
    let limits_map = cfg
        .extra
        .get("tokenLimits")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let contexts = state.hooks.context_by_id.lock().unwrap().clone();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let resets_at = now + 24 * 3600 * 1000;

    let mut agents_out = serde_json::Map::new();
    let mut office_used = 0u64;
    for id in &req.agent_ids {
        if id.trim().is_empty() {
            continue;
        }
        let lim = normalize_limit_entry(limits_map.get(id));
        let period = lim
            .as_ref()
            .and_then(|l| l.get("period"))
            .and_then(|p| p.as_str())
            .unwrap_or("day")
            .to_string();
        let used = contexts.get(id).map(|c| c.tokens).unwrap_or(0);
        office_used = office_used.saturating_add(used);
        agents_out.insert(
            id.clone(),
            json!({
                "limit": lim,
                "period": period,
                "used": used,
                "resetsAt": resets_at,
            }),
        );
    }

    let office_lim = normalize_limit_entry(limits_map.get("office"));
    let office_period = office_lim
        .as_ref()
        .and_then(|l| l.get("period"))
        .and_then(|p| p.as_str())
        .unwrap_or("day")
        .to_string();

    Json(json!({
        "office": {
            "limit": office_lim,
            "period": office_period,
            "used": office_used,
            "resetsAt": resets_at,
        },
        "agents": agents_out,
    }))
}

async fn memory_status(State(state): State<AppState>) -> Json<Value> {
    let home = state.harness_home();
    let cfg = state.config.lock().unwrap().read();
    let enabled = cfg
        .extra
        .get("semanticMemory")
        .or_else(|| cfg.extra.get("memoryEnabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let model = cfg
        .extra
        .get("embeddingModel")
        .or_else(|| cfg.extra.get("memoryModel"))
        .and_then(|v| v.as_str());
    let st = memory::memory_status(home.as_deref(), enabled, model);
    Json(serde_json::to_value(st).unwrap_or(Value::Null))
}

async fn memory_search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Json<Value> {
    let Some(home) = state.harness_home() else {
        return Json(json!([]));
    };
    let hits = memory::search_memory(&home, &q.q, q.limit.unwrap_or(20));
    Json(serde_json::to_value(hits).unwrap_or(Value::Null))
}

async fn memory_build_index(State(state): State<AppState>) -> Json<Value> {
    let Some(home) = state.harness_home() else {
        return Json(json!({ "ok": false, "error": "harnessHome is not set" }));
    };
    match memory::write_memory_index(&home) {
        Ok(path) => Json(json!({ "ok": true, "path": path })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn memory_get_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Json<Value> {
    let Some(home) = state.harness_home() else {
        return Json(json!({ "ok": false, "error": "harnessHome is not set" }));
    };
    match memory::read_agent_memory(&home, &agent_id) {
        Ok(content) => Json(json!({ "ok": true, "agentId": agent_id, "content": content })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn memory_put_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<MemoryWriteReq>,
) -> Json<Value> {
    let Some(home) = state.harness_home() else {
        return Json(json!({ "ok": false, "error": "harnessHome is not set" }));
    };
    match memory::write_agent_memory(&home, &agent_id, &req.content) {
        Ok(path) => Json(json!({ "ok": true, "path": path })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn kg_status(State(state): State<AppState>) -> Json<Value> {
    let (root, enabled) = resolve_kg_root(&state);
    Json(serde_json::to_value(kg::status(&root, enabled)).unwrap_or(Value::Null))
}

async fn kg_list(State(state): State<AppState>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    Json(serde_json::to_value(kg::list(&root)).unwrap_or(Value::Null))
}

async fn kg_search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    Json(serde_json::to_value(kg::search(&root, &q.q, q.limit.unwrap_or(8))).unwrap_or(Value::Null))
}

async fn kg_ingest(State(state): State<AppState>, Json(req): Json<KgIngestInput>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    match kg::ingest(&root, req) {
        Ok(meta) => Json(json!({ "ok": true, "docId": meta.id, "meta": meta })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn kg_get_doc(State(state): State<AppState>, Path(doc_id): Path<String>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    match kg::get_doc(&root, &doc_id) {
        Some((meta, text)) => Json(json!({ "ok": true, "meta": meta, "text": text })),
        None => Json(json!({ "ok": false, "error": "not found" })),
    }
}

async fn kg_delete_doc(State(state): State<AppState>, Path(doc_id): Path<String>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    let removed = kg::remove_doc(&root, &doc_id);
    Json(json!({ "ok": removed }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolveSessionQuery {
    session_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KgUploadReq {
    file_name: String,
    text_base64: Option<String>,
    text: Option<String>,
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn find_claude_session_cwd(session_id: &str) -> Option<String> {
    let root = default_projects_root();
    let rd = std::fs::read_dir(&root).ok()?;
    let target = format!("{session_id}.jsonl");
    for entry in rd.flatten() {
        let candidate = entry.path().join(&target);
        if !candidate.is_file() {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&candidate) else {
            continue;
        };
        for line in raw.lines().take(20) {
            if let Ok(v) = serde_json::from_str::<Value>(line)
                && let Some(cwd) = v.get("cwd").and_then(|c| c.as_str())
                && !cwd.is_empty()
            {
                return Some(cwd.to_string());
            }
        }
    }
    None
}

fn find_opencode_session_cwd(session_id: &str) -> Option<String> {
    let db_path = dirs::home_dir()?.join(".local/share/opencode/opencode.db");
    if !db_path.is_file() {
        return None;
    }
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    let mut stmt = conn
        .prepare("SELECT directory FROM session WHERE id = ?1 LIMIT 1")
        .ok()?;
    stmt.query_row([session_id], |r| r.get::<_, String>(0)).ok()
}

async fn usage_resolve_session(Query(q): Query<ResolveSessionQuery>) -> Json<Value> {
    let sid = q.session_id.trim();
    if sid.is_empty() {
        return Json(Value::Null);
    }
    let cwd = find_claude_session_cwd(sid).or_else(|| find_opencode_session_cwd(sid));
    match cwd {
        Some(c) => Json(json!({ "cwd": c, "sessionId": sid })),
        None => Json(Value::Null),
    }
}

async fn usage_limits(State(state): State<AppState>) -> Json<Value> {
    let contexts = state.hooks.context_by_id.lock().unwrap().clone();
    let home = state.harness_home();
    let mut claude_used = 0u64;
    let mut codex_used = 0u64;
    let mut opencode_used = 0u64;
    if let Some(snap) = state.roster.lock().unwrap().read(home.as_deref()) {
        for a in snap.agents {
            let Some(id) = a.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let tokens = contexts.get(id).map(|c| c.tokens).unwrap_or(0);
            match a.get("provider").and_then(|v| v.as_str()).unwrap_or("claude") {
                "codex" => codex_used = codex_used.saturating_add(tokens),
                "opencode" => opencode_used = opencode_used.saturating_add(tokens),
                _ => claude_used = claude_used.saturating_add(tokens),
            }
        }
    }
    Json(json!({
        "engines": [
            {
                "engine": "claude",
                "id": "claude",
                "name": "Claude Code",
                "state": "not-available",
                "plan": Value::Null,
                "failed": false,
                "asOf": Value::Null,
                "source": "local",
                "used": claude_used,
                "limit": Value::Null,
                "windows": [],
            },
            {
                "engine": "codex",
                "id": "codex",
                "name": "Codex",
                "state": "not-available",
                "plan": Value::Null,
                "failed": false,
                "asOf": Value::Null,
                "source": "local",
                "used": codex_used,
                "limit": Value::Null,
                "windows": [],
            },
            {
                "engine": "opencode",
                "id": "opencode",
                "name": "OpenCode",
                "state": "not-available",
                "plan": Value::Null,
                "failed": false,
                "asOf": Value::Null,
                "source": "local",
                "used": opencode_used,
                "limit": Value::Null,
                "windows": [],
            },
        ]
    }))
}

async fn kg_upload(State(state): State<AppState>, Json(req): Json<KgUploadReq>) -> Json<Value> {
    let (root, _) = resolve_kg_root(&state);
    let safe_name: String = PathBuf::from(&req.file_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("upload.txt")
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let title = req
        .title
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| safe_name.clone());

    let input = if let Some(text) = req.text {
        KgIngestInput {
            text: Some(text),
            title: Some(title),
            source: Some(safe_name),
            tags: req.tags,
            ..Default::default()
        }
    } else if let Some(b64) = req.text_base64 {
        let raw_b64 = b64
            .split_once(";base64,")
            .map(|(_, rest)| rest)
            .unwrap_or(b64.as_str())
            .trim();
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw_b64) else {
            return Json(json!({ "ok": false, "error": "invalid base64 payload" }));
        };
        let upload_dir = root.join("uploads");
        if let Err(e) = std::fs::create_dir_all(&upload_dir) {
            return Json(json!({ "ok": false, "error": e.to_string() }));
        }
        let dest = upload_dir.join(format!("{}-{safe_name}", uuid::Uuid::new_v4().simple()));
        if let Err(e) = std::fs::write(&dest, &bytes) {
            return Json(json!({ "ok": false, "error": e.to_string() }));
        }
        let text_fallback = String::from_utf8(bytes).ok();
        KgIngestInput {
            src_path: if text_fallback.is_none() {
                Some(dest.to_string_lossy().into_owned())
            } else {
                None
            },
            text: text_fallback,
            title: Some(title),
            source: Some(safe_name),
            tags: req.tags,
            ..Default::default()
        }
    } else {
        return Json(json!({ "ok": false, "error": "missing text or textBase64" }));
    };

    match kg::ingest(&root, input) {
        Ok(meta) => Json(json!({ "ok": true, "docId": meta.id, "meta": meta })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

pub fn usage_memory_kg_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/history/page", get(history_page))
        .route("/api/v1/usage/agent", get(usage_agent))
        .route("/api/v1/usage/context", get(usage_context))
        .route("/api/v1/usage/resolve-session", get(usage_resolve_session))
        .route("/api/v1/usage/limits", get(usage_limits))
        .route("/api/v1/telemetry/snapshot", get(telemetry_snapshot))
        .route("/api/v1/telemetry/usage", get(telemetry_usage))
        .route("/api/v1/telemetry/spans", get(telemetry_spans))
        .route("/api/v1/config/token-limit", post(config_set_token_limit))
        .route("/api/v1/token-limits/view", post(token_limits_view))
        .route("/api/v1/memory/status", get(memory_status))
        .route("/api/v1/memory/search", get(memory_search))
        .route("/api/v1/memory/index", post(memory_build_index))
        .route(
            "/api/v1/memory/{agentId}",
            get(memory_get_agent).put(memory_put_agent),
        )
        .route("/api/v1/kg/status", get(kg_status))
        .route("/api/v1/kg/list", get(kg_list))
        .route("/api/v1/kg/search", get(kg_search))
        .route("/api/v1/kg/ingest", post(kg_ingest))
        .route("/api/v1/kg/upload", post(kg_upload))
        .route(
            "/api/v1/kg/docs/{id}",
            get(kg_get_doc).delete(kg_delete_doc),
        )
}
