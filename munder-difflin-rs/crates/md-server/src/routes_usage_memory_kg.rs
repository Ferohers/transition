//! HTTP routes for Phase 5: local transcript usage, memory manager, and Knowledge Graph.

use crate::state::AppState;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use md_core::config::{default_data_dir, expand_tilde};
use md_hive::{
    kg::{self, KgIngestInput},
    memory,
    usage::{default_projects_root, read_agent_usage_in, read_context_tokens},
};
use serde::Deserialize;
use serde_json::{Value, json};
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
    limit: Option<u64>,
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

fn resolve_kg_root(state: &AppState) -> (PathBuf, bool) {
    let mut cfg_store = state.config.lock().unwrap();
    let cfg = cfg_store.read();
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
            state
                .harness_home()
                .map(|h| h.join("knowledge"))
                .unwrap_or_else(|| default_data_dir().join("knowledge"))
        });
    (root, enabled)
}

async fn usage_agent(Query(q): Query<UsageQuery>) -> Json<Value> {
    let root = default_projects_root();
    let res = read_agent_usage_in(&root, &q.cwd, q.session_id.as_deref());
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
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
    if let Some(lim) = req.limit {
        limits.insert(req.agent_id, json!(lim));
    } else {
        limits.remove(&req.agent_id);
    }
    match cfg_store.update(json!({ "tokenLimits": limits })) {
        Ok(next) => Json(json!({ "ok": true, "tokenLimits": next.extra.get("tokenLimits") })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

async fn memory_status(State(state): State<AppState>) -> Json<Value> {
    let home = state.harness_home();
    let cfg = state.config.lock().unwrap().read();
    let enabled = cfg
        .extra
        .get("memoryEnabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let model = cfg.extra.get("memoryModel").and_then(|v| v.as_str());
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

pub fn usage_memory_kg_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/usage/agent", get(usage_agent))
        .route("/api/v1/usage/context", get(usage_context))
        .route("/api/v1/config/token-limit", post(config_set_token_limit))
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
        .route(
            "/api/v1/kg/docs/{id}",
            get(kg_get_doc).delete(kg_delete_doc),
        )
}
