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
use std::sync::Mutex;
use std::time::SystemTime;

type OcLimitsEntry = (PathBuf, Option<SystemTime>, HashMap<String, u64>);

static OC_LIMITS_CACHE: Mutex<Option<OcLimitsEntry>> = Mutex::new(None);

fn parse_opencode_models_json(raw: &str) -> HashMap<String, u64> {
    let mut map = HashMap::new();
    let Ok(Value::Object(providers)) = serde_json::from_str::<Value>(raw) else {
        return map;
    };
    for (prov_id, prov_val) in &providers {
        let Some(models) = prov_val.get("models").and_then(|v| v.as_object()) else {
            continue;
        };
        for (model_id, model_val) in models {
            let Some(ctx_limit) = model_val
                .get("limit")
                .and_then(|l| l.get("context"))
                .and_then(|v| v.as_u64())
                .filter(|&n| n > 0)
            else {
                continue;
            };
            map.insert(format!("{prov_id}/{model_id}"), ctx_limit);
            if prov_id == "opencode" || prov_id == "anthropic" || !map.contains_key(model_id) {
                map.insert(model_id.clone(), ctx_limit);
            }
        }
    }
    map
}

pub(crate) fn opencode_model_context_limit_from_path(
    models_path: &std::path::Path,
    provider_id: Option<&str>,
    model_id: &str,
) -> u64 {
    let mtime = std::fs::metadata(models_path).and_then(|m| m.modified()).ok();
    let mut cache = OC_LIMITS_CACHE.lock().unwrap();
    let stale = match cache.as_ref() {
        Some((p, t, _)) => p != models_path || *t != mtime,
        None => true,
    };
    if stale {
        let parsed = std::fs::read_to_string(models_path)
            .map(|s| parse_opencode_models_json(&s))
            .unwrap_or_default();
        *cache = Some((models_path.to_path_buf(), mtime, parsed));
    }
    let map = &cache.as_ref().unwrap().2;
    let trimmed = model_id.trim();
    if let Some(prov) = provider_id.map(str::trim).filter(|s| !s.is_empty())
        && let Some(&limit) = map.get(&format!("{prov}/{trimmed}"))
    {
        return limit;
    }
    if let Some(&limit) = map.get(trimmed) {
        return limit;
    }
    if let Some((_, short)) = trimmed.split_once('/')
        && let Some(&limit) = map.get(short)
    {
        return limit;
    }
    if trimmed.to_ascii_lowercase().contains("1m") {
        1_000_000
    } else {
        200_000
    }
}

pub(crate) fn opencode_model_context_limit(provider_id: Option<&str>, model_id: &str) -> u64 {
    let cache_base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cache")));
    let Some(base) = cache_base else {
        return if model_id.to_ascii_lowercase().contains("1m") {
            1_000_000
        } else {
            200_000
        };
    };
    opencode_model_context_limit_from_path(
        &base.join("opencode").join("models.json"),
        provider_id,
        model_id,
    )
}

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

pub(crate) fn read_opencode_usage_for_since(
    cwd: &str,
    session_id: Option<&str>,
    allow_cwd_fallback: bool,
    min_updated_ms: Option<u64>,
) -> Option<(md_hive::usage::AgentUsage, Option<u64>)> {
    let home = dirs::home_dir()?;
    let db_path = home.join(".local").join("share").join("opencode").join("opencode.db");
    read_opencode_usage_from_db(&db_path, cwd, session_id, allow_cwd_fallback, min_updated_ms)
}

fn read_opencode_usage_from_db(
    db_path: &std::path::Path,
    cwd: &str,
    session_id: Option<&str>,
    allow_cwd_fallback: bool,
    min_updated_ms: Option<u64>,
) -> Option<(md_hive::usage::AgentUsage, Option<u64>)> {
    if !db_path.is_file() {
        return None;
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(db_path, flags).ok()?;
    let (ses_id, ses_updated): (String, i64) = if let Some(sid) = session_id.filter(|s| !s.is_empty())
    {
        let mut stmt = conn
            .prepare("SELECT id, time_updated FROM session WHERE id = ?1 LIMIT 1")
            .ok()?;
        stmt.query_row([sid], |r| Ok((r.get(0)?, r.get(1)?))).ok()?
    } else if allow_cwd_fallback && !cwd.is_empty() {
        if let Some(min_ms) = min_updated_ms {
            let cutoff = i64::try_from(min_ms.saturating_sub(5_000)).unwrap_or(i64::MAX);
            let mut stmt = conn
                .prepare("SELECT id, time_updated FROM session WHERE directory = ?1 AND time_updated >= ?2 ORDER BY time_updated DESC LIMIT 1")
                .ok()?;
            stmt.query_row(rusqlite::params![cwd, cutoff], |r| Ok((r.get(0)?, r.get(1)?))).ok()?
        } else {
            let mut stmt = conn
                .prepare("SELECT id, time_updated FROM session WHERE directory = ?1 ORDER BY time_updated DESC LIMIT 1")
                .ok()?;
            stmt.query_row([cwd], |r| Ok((r.get(0)?, r.get(1)?))).ok()?
        }
    } else {
        return None;
    };

    let mut msg_stmt = conn
        .prepare("SELECT data, time_created FROM message WHERE session_id = ?1 ORDER BY time_created ASC")
        .ok()?;
    let rows = msg_stmt
        .query_map([ses_id], |r| {
            let data: String = r.get(0)?;
            let ts: i64 = r.get(1)?;
            Ok((data, ts))
        })
        .ok()?;

    let mut usage = md_hive::usage::AgentUsage {
        as_of: u64::try_from(ses_updated.max(0)).unwrap_or(0),
        ..Default::default()
    };
    let mut latest_context: Option<u64> = None;
    for row in rows.flatten() {
        let Ok(v) = serde_json::from_str::<Value>(&row.0) else {
            continue;
        };
        if v.get("role").and_then(|x| x.as_str()) != Some("assistant") {
            continue;
        }
        if let Some(m) = v.get("modelID").and_then(|x| x.as_str()) {
            usage.model = Some(m.to_string());
        }
        if let Some(c) = v.get("cost").and_then(|x| x.as_f64()) {
            usage.estimated_cost_usd += c;
        }
        if let Some(t) = v.get("tokens") {
            let inp = t.get("input").and_then(|x| x.as_u64()).unwrap_or(0);
            let out = t.get("output").and_then(|x| x.as_u64()).unwrap_or(0);
            let cw = t
                .get("cache")
                .and_then(|c| c.get("write"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let cr = t
                .get("cache")
                .and_then(|c| c.get("read"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            usage.input_tokens = usage.input_tokens.saturating_add(inp);
            usage.output_tokens = usage.output_tokens.saturating_add(out);
            usage.cache_write_tokens = usage.cache_write_tokens.saturating_add(cw);
            usage.cache_read_tokens = usage.cache_read_tokens.saturating_add(cr);
            let step_done = v
                .get("time")
                .and_then(|tm| tm.get("completed"))
                .is_some_and(|x| !x.is_null())
                || v.get("finish").and_then(|x| x.as_str()).is_some_and(|s| !s.is_empty());
            if step_done {
                if let Some(total) = t.get("total").and_then(|x| x.as_u64()).filter(|&n| n > 0) {
                    latest_context = Some(total);
                } else {
                    let sum = inp.saturating_add(cr).saturating_add(out).saturating_add(cw);
                    if sum > 0 {
                        latest_context = Some(sum);
                    }
                }
            }
        }
    }
    Some((usage, latest_context))
}

async fn usage_agent(State(state): State<AppState>, Query(q): Query<UsageQuery>) -> Json<Value> {
    let root = default_projects_root();
    let expanded_cwd = expand_tilde(&q.cwd).to_string_lossy().into_owned();
    let (hook_sid, started_at): (Option<String>, Option<u64>) = {
        let reg = state.hive.lock().unwrap().registry();
        let sessions = state.hooks.session_by_id.lock().unwrap().clone();
        let mut matched = (None, None);
        if let Some(agents) = reg.get("agents").and_then(|v| v.as_object()) {
            for (aid, info) in agents {
                if info.get("archived").and_then(|v| v.as_bool()).unwrap_or(false) {
                    continue;
                }
                let acwd = info
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(expand_tilde)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if acwd == expanded_cwd {
                    matched = (
                        sessions.get(aid).cloned(),
                        info.get("startedAt").and_then(|v| v.as_u64()),
                    );
                    if aid == "god" {
                        break;
                    }
                }
            }
        }
        matched
    };
    let eff_sid = q.session_id.as_deref().or(hook_sid.as_deref());
    let res = read_agent_usage_in(&root, &q.cwd, eff_sid);
    let total = res.input_tokens + res.output_tokens + res.cache_read_tokens + res.cache_write_tokens;
    if total == 0
        && let Some((oc, _)) =
            read_opencode_usage_for_since(&expanded_cwd, eff_sid, true, started_at)
    {
        return Json(serde_json::to_value(oc).unwrap_or(Value::Null));
    }
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
}

pub(crate) fn collect_agent_usage_rows(
    state: &AppState,
) -> Vec<(String, md_hive::usage::AgentUsage)> {
    let home = state.harness_home();
    let root = default_projects_root();
    let snap = state.roster.lock().unwrap().read(home.as_deref());
    let reg = state.hive.lock().unwrap().registry();
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
            let provider = a.get("provider").and_then(|v| v.as_str()).unwrap_or("");
            let sid = a.get("sessionId").and_then(|v| v.as_str());
            let sleeping = a.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false);
            let hook_sid = state
                .hooks
                .session_by_id
                .lock()
                .unwrap()
                .get(id)
                .cloned();
            let started_at = reg
                .get("agents")
                .and_then(|m| m.get(id))
                .and_then(|info| info.get("startedAt"))
                .and_then(|v| v.as_u64());
            let eff_sid = sid.or(hook_sid.as_deref());
            let mut u = if provider == "opencode" {
                read_opencode_usage_for_since(&cwd, eff_sid, !sleeping, started_at)
                    .map(|(oc, _)| oc)
                    .unwrap_or_default()
            } else {
                read_agent_usage_in(&root, &cwd, eff_sid)
            };
            if u.input_tokens + u.output_tokens + u.cache_read_tokens + u.cache_write_tokens == 0
                && provider != "opencode"
                && !sleeping
                && let Some((oc, _)) = read_opencode_usage_for_since(&cwd, eff_sid, true, started_at)
            {
                u = oc;
            }
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
    let usage_totals: HashMap<String, u64> = collect_agent_usage_rows(&state)
        .into_iter()
        .map(|(id, u)| {
            (
                id,
                u.input_tokens
                    .saturating_add(u.output_tokens)
                    .saturating_add(u.cache_write_tokens)
                    .saturating_add(u.cache_read_tokens),
            )
        })
        .collect();
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
        let used = contexts
            .get(id)
            .map(|c| c.tokens)
            .or_else(|| usage_totals.get(id).copied())
            .unwrap_or(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_usage_respects_min_updated_ms_and_session_id() {
        let dir = std::env::temp_dir().join(format!(
            "md-oc-usage-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("opencode.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL, time_updated INTEGER NOT NULL);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session (id, directory, time_updated) VALUES (?1, ?2, ?3)",
            rusqlite::params!["ses-old", "/tmp/acme", 1_000_000i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "m-old",
                "ses-old",
                1_000_000i64,
                r#"{"role":"assistant","modelID":"m1","cost":0.1,"finish":"stop","time":{"completed":1000500},"tokens":{"input":120000,"output":500,"cache":{"read":18000,"write":0},"total":138500}}"#
            ],
        )
        .unwrap();

        // Fallback without session_id is blocked when session predates agent launch (`min_updated_ms`).
        assert!(read_opencode_usage_from_db(&db_path, "/tmp/acme", None, true, Some(2_000_000)).is_none());
        // Exact session_id still resolves if explicitly provided.
        let (u_exact, ctx_exact) =
            read_opencode_usage_from_db(&db_path, "/tmp/acme", Some("ses-old"), true, Some(2_000_000))
                .unwrap();
        assert_eq!(u_exact.input_tokens, 120_000);
        assert_eq!(ctx_exact, Some(138_500));

        // New session updated after agent start is picked up normally.
        conn.execute(
            "INSERT INTO session (id, directory, time_updated) VALUES (?1, ?2, ?3)",
            rusqlite::params!["ses-new", "/tmp/acme", 2_005_000i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "m-new",
                "ses-new",
                2_005_000i64,
                r#"{"role":"assistant","modelID":"m2","cost":0.01,"finish":"stop","time":{"completed":2005500},"tokens":{"input":400,"output":50,"total":450}}"#
            ],
        )
        .unwrap();
        // Mid-stream incomplete step must not overwrite completed context tokens.
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "m-streaming",
                "ses-new",
                2_006_000i64,
                r#"{"role":"assistant","modelID":"m2","tokens":{"input":12,"output":0,"total":12}}"#
            ],
        )
        .unwrap();
        let (u_new, ctx_new) =
            read_opencode_usage_from_db(&db_path, "/tmp/acme", None, true, Some(2_000_000))
                .unwrap();
        assert_eq!(u_new.input_tokens, 412);
        assert_eq!(ctx_new, Some(450));

        let models_path = dir.join("models.json");
        std::fs::write(
            &models_path,
            serde_json::json!({
                "opencode": {
                    "models": {
                        "hy4-flash-free": { "limit": { "context": 1_048_576, "output": 65_536 } },
                        "kimi-k2.5-free": { "limit": { "context": 262_144, "output": 262_144 } }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            opencode_model_context_limit_from_path(&models_path, Some("opencode"), "hy4-flash-free"),
            1_048_576
        );
        assert_eq!(
            opencode_model_context_limit_from_path(&models_path, None, "kimi-k2.5-free"),
            262_144
        );
        assert_eq!(
            opencode_model_context_limit_from_path(&models_path, None, "unknown-model"),
            200_000
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
