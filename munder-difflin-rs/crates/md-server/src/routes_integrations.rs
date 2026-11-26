//! HTTP routes for Phase 6: write-only secrets, integration broker,
//! generic webhooks, local HTTP file share, and Realtime Michael token mint.

use crate::state::AppState;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use md_integrations::{
    file_share::ShareCreateReq,
    secrets::IntegrationRecord,
    voice::{has_openai_key, mint_realtime_token},
    webhooks::{WebhookEndpointInput, WebhookInboundPayload},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSecretReq {
    key_ref: String,
    #[serde(default = "default_secret_kind")]
    kind: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CustomSecretSetEntry {
    name: String,
    value: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplySecretsReq {
    #[serde(default)]
    set: Vec<CustomSecretSetEntry>,
    #[serde(default)]
    remove: Vec<String>,
}

fn default_secret_kind() -> String {
    "custom".to_string()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrantCapReq {
    worker_id: String,
    #[serde(default)]
    allowed_ids: Vec<String>,
    #[serde(default = "default_true")]
    allow_secrets_read: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerSecretReadReq {
    key_ref: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebhookPollQuery {
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DecideTriggerReq {
    decision: String,
    task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharePasswordQuery {
    password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MintRealtimeReq {
    model: Option<String>,
}

fn extract_broker_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(v) = headers.get("x-md-broker-token").and_then(|h| h.to_str().ok()) {
        return Some(v.trim());
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(str::trim)
}

async fn secrets_list(State(state): State<AppState>) -> Json<Value> {
    let list = state.secrets.lock().unwrap().list_metadata();
    Json(serde_json::to_value(list).unwrap_or(Value::Null))
}

async fn secrets_set(State(state): State<AppState>, Json(req): Json<SetSecretReq>) -> Json<Value> {
    match state
        .secrets
        .lock()
        .unwrap()
        .set_secret(&req.key_ref, &req.kind, &req.value)
    {
        Ok(()) => Json(json!({ "ok": true, "keyRef": req.key_ref })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn secrets_delete(State(state): State<AppState>, Path(key_ref): Path<String>) -> Json<Value> {
    match state.secrets.lock().unwrap().delete_secret(&key_ref) {
        Ok(removed) => Json(json!({ "ok": removed })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn secrets_apply(
    State(state): State<AppState>,
    Json(req): Json<ApplySecretsReq>,
) -> Json<Value> {
    let mut secrets = state.secrets.lock().unwrap();
    for item in req.set {
        if let Err(error) = secrets.set_secret(&item.name, "custom", &item.value) {
            return Json(json!({ "ok": false, "code": "bad_value", "name": item.name, "error": error }));
        }
    }
    for name in req.remove {
        if let Err(error) = secrets.delete_secret(&name) {
            return Json(json!({ "ok": false, "code": "bad_value", "name": name, "error": error }));
        }
    }
    Json(json!({ "ok": true }))
}

async fn broker_integrations_list(State(state): State<AppState>) -> Json<Value> {
    let records: Vec<IntegrationRecord> = {
        let broker = state.broker.lock().unwrap();
        broker.list_records().into_iter().cloned().collect()
    };
    let secrets = state.secrets.lock().unwrap();
    let enriched: Vec<Value> = records
        .into_iter()
        .map(|rec| {
            let has_secret = rec
                .secret_ref
                .as_deref()
                .map(|r| secrets.has_secret(r))
                .unwrap_or(false);
            let mut val = serde_json::to_value(&rec).unwrap_or(Value::Null);
            if let Some(obj) = val.as_object_mut() {
                obj.insert("hasSecret".to_string(), Value::Bool(has_secret));
            }
            val
        })
        .collect();
    Json(Value::Array(enriched))
}

async fn broker_integrations_upsert(
    State(state): State<AppState>,
    Json(rec): Json<IntegrationRecord>,
) -> Json<Value> {
    let id = rec.id.clone();
    match state.broker.lock().unwrap().upsert_record(rec) {
        Ok(()) => {
            push_activity(&state, "integration:upsert", &id, json!({ "ok": true }));
            Json(json!({ "ok": true, "id": id }))
        }
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveIntegrationReq {
    record: IntegrationRecord,
    secret: Option<String>,
}

async fn broker_integrations_save(
    State(state): State<AppState>,
    Json(req): Json<SaveIntegrationReq>,
) -> Json<Value> {
    let id = req.record.id.clone();
    let secret_ref = req
        .record
        .secret_ref
        .clone()
        .unwrap_or_else(|| format!("int:{id}"));
    if let Err(error) = state.broker.lock().unwrap().upsert_record(req.record) {
        return Json(json!({ "ok": false, "error": error }));
    }
    if let Some(secret) = req.secret.filter(|s| !s.is_empty())
        && let Err(error) = state
            .secrets
            .lock()
            .unwrap()
            .set_secret(&secret_ref, "integration", &secret)
    {
        return Json(json!({ "ok": false, "error": error }));
    }
    push_activity(&state, "integration:save", &id, json!({ "ok": true }));
    Json(json!({ "ok": true, "id": id }))
}

async fn broker_integrations_remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Json<Value> {
    let (removed, secret_ref) = {
        let mut broker = state.broker.lock().unwrap();
        let sr = broker
            .get_record(&id)
            .and_then(|r| r.secret_ref.clone())
            .unwrap_or_else(|| format!("int:{id}"));
        match broker.remove_record(&id) {
            Ok(r) => (r, sr),
            Err(error) => return Json(json!({ "ok": false, "error": error })),
        }
    };
    let _ = state.secrets.lock().unwrap().delete_secret(&secret_ref);
    if removed {
        push_activity(&state, "integration:remove", &id, json!({ "ok": true }));
    }
    Json(json!({ "ok": removed }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TestIntegrationReq {
    id: Option<String>,
    draft: Option<IntegrationRecord>,
    secret: Option<String>,
}

async fn broker_integrations_test(
    State(state): State<AppState>,
    Json(req): Json<TestIntegrationReq>,
) -> Json<Value> {
    let (record, secret) = if let Some(draft) = req.draft {
        let sec = req.secret.filter(|s| !s.is_empty()).or_else(|| {
            draft
                .secret_ref
                .as_deref()
                .and_then(|r| state.secrets.lock().unwrap().reveal_for_broker(r))
        });
        (draft, sec)
    } else if let Some(id) = req.id {
        let rec = {
            let broker = state.broker.lock().unwrap();
            broker.get_record(&id).cloned()
        };
        let Some(rec) = rec else {
            return Json(json!({ "ok": false, "error": "unknown integration" }));
        };
        if !rec.enabled {
            return Json(json!({ "ok": false, "error": "integration is disabled" }));
        }
        let sec = rec
            .secret_ref
            .as_deref()
            .and_then(|r| state.secrets.lock().unwrap().reveal_for_broker(r));
        (rec, sec)
    } else {
        return Json(json!({ "ok": false, "error": "missing integration id or draft" }));
    };

    let res = md_integrations::secrets::IntegrationBroker::test_connection(
        &record,
        secret.as_deref(),
    )
    .await;
    push_activity(
        &state,
        "integration:test",
        &record.id,
        json!({ "ok": res.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) }),
    );
    Json(res)
}

async fn broker_grant_cap(
    State(state): State<AppState>,
    Json(req): Json<GrantCapReq>,
) -> Json<Value> {
    let tok = state.broker.lock().unwrap().grant_capability(
        &req.worker_id,
        req.allowed_ids,
        req.allow_secrets_read,
    );
    Json(json!({ "ok": true, "token": tok }))
}

async fn broker_secrets_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    let Some(tok) = extract_broker_token(&headers) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    {
        let broker = state.broker.lock().unwrap();
        if broker.validate_token(tok).is_none() {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    let meta = state.secrets.lock().unwrap().list_metadata();
    Ok(Json(serde_json::to_value(meta).unwrap_or(Value::Null)))
}

async fn broker_secrets_read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<BrokerSecretReadReq>,
) -> Result<Json<Value>, StatusCode> {
    let Some(tok) = extract_broker_token(&headers) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    {
        let broker = state.broker.lock().unwrap();
        let Some(cap) = broker.validate_token(tok) else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if !cap.allow_secrets_read {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    let value = state.secrets.lock().unwrap().reveal_for_broker(&req.key_ref);
    Ok(Json(json!({ "ok": value.is_some(), "value": value })))
}

async fn webhooks_list(State(state): State<AppState>) -> Json<Value> {
    let list = state.webhooks.lock().unwrap().list_endpoints();
    Json(serde_json::to_value(list).unwrap_or(Value::Null))
}

async fn webhooks_upsert(
    State(state): State<AppState>,
    Json(req): Json<WebhookEndpointInput>,
) -> Json<Value> {
    match state.webhooks.lock().unwrap().upsert_endpoint(req) {
        Ok(ep) => {
            state.emit("webhooks:changed", json!({ "id": ep.id }));
            Json(json!({ "ok": true, "endpoint": ep }))
        }
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn webhooks_save(
    State(state): State<AppState>,
    Json(list): Json<Vec<WebhookEndpointInput>>,
) -> Json<Value> {
    let saved = state.webhooks.lock().unwrap().replace_endpoints(list);
    state.emit("webhooks:changed", json!({ "count": saved.len() }));
    Json(serde_json::to_value(saved).unwrap_or(Value::Array(Vec::new())))
}

async fn webhooks_status(State(state): State<AppState>) -> Json<Value> {
    let port: u16 = std::env::var("MD_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3110);
    let base_url = format!("http://127.0.0.1:{port}");
    let list = state.webhooks.lock().unwrap().list_endpoints();
    let endpoints: Vec<Value> = list
        .iter()
        .map(|ep| json!({ "id": ep.id, "url": format!("{base_url}/webhooks/{}", ep.id) }))
        .collect();
    Json(json!({
        "running": true,
        "starting": false,
        "url": base_url,
        "port": port,
        "endpoints": endpoints,
    }))
}

async fn webhooks_generate_secret() -> Json<Value> {
    let s = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    Json(json!({ "secret": s }))
}

async fn webhooks_retry() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn webhooks_delete(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let removed = state.webhooks.lock().unwrap().delete_endpoint(&id);
    if removed {
        state.emit("webhooks:changed", json!({ "id": id, "deleted": true }));
    }
    Json(json!({ "ok": removed }))
}

async fn webhooks_inbound(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<WebhookInboundPayload>,
) -> (StatusCode, Json<Value>) {
    let secret = headers
        .get("x-md-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let res = state
        .webhooks
        .lock()
        .unwrap()
        .accept_inbound(&id, secret, payload);
    match res {
        Ok(entry) => {
            let pending = entry.decision == "pending";
            let status = if pending {
                StatusCode::ACCEPTED
            } else {
                StatusCode::OK
            };
            state.emit(
                "triggers:inbound",
                serde_json::to_value(&entry).unwrap_or(Value::Null),
            );
            push_activity(
                &state,
                "webhook:inbound",
                &id,
                json!({ "entryId": entry.id, "status": entry.decision, "pending": pending }),
            );
            (
                status,
                Json(json!({
                    "ok": true,
                    "pending": pending,
                    "token": entry.token,
                    "entryId": entry.id,
                    "status": entry.decision
                })),
            )
        }
        Err(_) => (StatusCode::UNAUTHORIZED, Json(json!({ "ok": false, "error": "unauthorized" }))),
    }
}

async fn webhooks_poll(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<WebhookPollQuery>,
) -> (StatusCode, Json<Value>) {
    let tok = headers
        .get("x-md-webhook-token")
        .and_then(|v| v.to_str().ok())
        .or(q.token.as_deref())
        .unwrap_or("");
    let wh = state.webhooks.lock().unwrap();
    match wh.lookup_by_token(&id, tok) {
        Some(entry) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "status": entry.decision,
                "title": entry.title,
                "taskId": entry.task_id
            })),
        ),
        None => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": "not_found" }))),
    }
}

async fn triggers_history(State(state): State<AppState>) -> Json<Value> {
    let wh = state.webhooks.lock().unwrap();
    Json(serde_json::to_value(wh.list_history()).unwrap_or(Value::Null))
}

async fn triggers_decide(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<DecideTriggerReq>,
) -> Json<Value> {
    let updated = state
        .webhooks
        .lock()
        .unwrap()
        .decide(&id, &req.decision, req.task_id);
    match updated {
        Some(entry) => Json(json!({ "ok": true, "entry": entry })),
        None => Json(json!({ "ok": false, "error": "not_found" })),
    }
}

async fn share_list(State(state): State<AppState>) -> Json<Value> {
    let list = state.file_share.lock().unwrap().list_shares();
    Json(serde_json::to_value(list).unwrap_or(Value::Null))
}

async fn share_create(State(state): State<AppState>, Json(req): Json<ShareCreateReq>) -> Json<Value> {
    match state.file_share.lock().unwrap().create_share(&req) {
        Ok(entry) => Json(json!({ "ok": true, "share": entry })),
        Err(error) => Json(json!({ "ok": false, "error": error })),
    }
}

async fn share_stop(State(state): State<AppState>, Path(id): Path<String>) -> Json<Value> {
    let stopped = state.file_share.lock().unwrap().stop_share(&id);
    Json(json!({ "ok": stopped }))
}

async fn share_stop_all(State(state): State<AppState>) -> Json<Value> {
    let stopped = state.file_share.lock().unwrap().stop_all();
    Json(json!({ "ok": true, "stopped": stopped }))
}

async fn share_download(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<SharePasswordQuery>,
) -> Response {
    let pass = headers
        .get("x-md-share-password")
        .and_then(|v| v.to_str().ok())
        .or(q.password.as_deref());
    let (abs_path, file_name) = {
        let reg = state.file_share.lock().unwrap();
        match reg.authorize(&id, pass) {
            Ok(entry) => (entry.abs_path.clone(), entry.file_name.clone()),
            Err("unauthorized") => return StatusCode::UNAUTHORIZED.into_response(),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let Ok(bytes) = tokio::fs::read(&abs_path).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut resp = Response::new(Body::from(bytes));
    if let Ok(val) = HeaderValue::from_str(&format!("attachment; filename=\"{file_name}\"")) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, val);
    }
    resp
}

async fn realtime_has_key(State(state): State<AppState>) -> Json<Value> {
    let has = has_openai_key(&state.secrets.lock().unwrap());
    Json(json!({ "hasKey": has }))
}

async fn realtime_mint_token(
    State(state): State<AppState>,
    Json(req): Json<MintRealtimeReq>,
) -> Json<Value> {
    let key = state
        .secrets
        .lock()
        .unwrap()
        .reveal_for_broker("apikey:openai");
    let res = mint_realtime_token(key, req.model.as_deref()).await;
    Json(serde_json::to_value(res).unwrap_or(Value::Null))
}

fn push_activity(state: &AppState, kind: &str, id: &str, detail: Value) {
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let entry = json!({
        "id": format!("{}-{at}", uuid::Uuid::new_v4().simple()),
        "kind": kind,
        "targetId": id,
        "at": at,
        "detail": detail,
    });
    {
        let mut act = state.integrations_activity.lock().unwrap();
        act.insert(0, entry.clone());
        if act.len() > 100 {
            act.truncate(100);
        }
    }
    state.emit("integrations:activity", entry);
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShareInspectReq {
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TelegramSetWebhookReq {
    token: Option<String>,
    webhook_url: Option<String>,
    secret_token: Option<String>,
}

async fn integrations_activity(State(state): State<AppState>) -> Json<Value> {
    let items = state.integrations_activity.lock().unwrap().clone();
    Json(Value::Array(items))
}

async fn share_inspect(Json(req): Json<ShareInspectReq>) -> Json<Value> {
    let out: Vec<Value> = req
        .paths
        .into_iter()
        .map(|p| {
            let expanded = md_core::config::expand_tilde(&p);
            match std::fs::metadata(&expanded) {
                Ok(meta) => {
                    let name = expanded
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("item")
                        .to_string();
                    let kind = if meta.is_dir() { "dir" } else { "file" };
                    json!({
                        "path": p,
                        "ok": true,
                        "name": name,
                        "kind": kind,
                        "size": meta.len(),
                    })
                }
                Err(e) => json!({
                    "path": p,
                    "ok": false,
                    "error": e.to_string(),
                }),
            }
        })
        .collect();
    Json(Value::Array(out))
}

async fn share_status(State(state): State<AppState>) -> Json<Value> {
    let list = state.file_share.lock().unwrap().list_shares();
    let out: Vec<Value> = list
        .into_iter()
        .map(|e| {
            let mut val = serde_json::to_value(&e).unwrap_or(Value::Null);
            if let Some(obj) = val.as_object_mut() {
                obj.insert("url".to_string(), json!(format!("/share/{}", e.id)));
            }
            val
        })
        .collect();
    Json(Value::Array(out))
}

async fn webhooks_telegram_set(
    State(state): State<AppState>,
    Json(req): Json<TelegramSetWebhookReq>,
) -> Json<Value> {
    if let Some(tok) = req.token.as_deref().map(str::trim).filter(|s| !s.is_empty())
        && let Err(e) = state
            .secrets
            .lock()
            .unwrap()
            .set_secret("apikey:telegram", "provider", tok)
    {
        return Json(json!({ "ok": false, "error": e }));
    }
    let Some(url) = req
        .webhook_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Json(json!({ "ok": false, "error": "webhookUrl is required" }));
    };
    push_activity(
        &state,
        "telegram:webhook",
        "telegram",
        json!({ "webhookUrl": url, "hasSecretToken": req.secret_token.is_some() }),
    );
    Json(json!({ "ok": true, "webhookUrl": url }))
}

pub fn integrations_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/secrets", get(secrets_list).post(secrets_set))
        .route("/api/v1/secrets/apply", post(secrets_apply))
        .route("/api/v1/secrets/{keyRef}", delete(secrets_delete))
        .route(
            "/api/v1/integrations",
            get(broker_integrations_list).post(broker_integrations_upsert),
        )
        .route("/api/v1/integrations/save", post(broker_integrations_save))
        .route("/api/v1/integrations/test", post(broker_integrations_test))
        .route("/api/v1/integrations/activity", get(integrations_activity))
        .route("/api/v1/integrations/{id}", delete(broker_integrations_remove))
        .route("/api/v1/broker/capabilities", post(broker_grant_cap))
        .route("/broker/secrets", get(broker_secrets_list).post(broker_secrets_list))
        .route("/broker/secrets/read", post(broker_secrets_read))
        .route(
            "/api/v1/webhooks/endpoints",
            get(webhooks_list).post(webhooks_upsert),
        )
        .route("/api/v1/webhooks/save", post(webhooks_save))
        .route("/api/v1/webhooks/status", get(webhooks_status))
        .route("/api/v1/webhooks/generate-secret", post(webhooks_generate_secret))
        .route("/api/v1/webhooks/retry", post(webhooks_retry))
        .route("/api/v1/webhooks/telegram-set", post(webhooks_telegram_set))
        .route("/api/v1/webhooks/endpoints/{id}", delete(webhooks_delete))
        .route("/webhooks/{id}", post(webhooks_inbound).get(webhooks_poll))
        .route("/api/v1/triggers/history", get(triggers_history))
        .route("/api/v1/triggers/history/{id}/decide", post(triggers_decide))
        .route("/api/v1/share", get(share_list).post(share_create))
        .route("/api/v1/share/inspect", post(share_inspect))
        .route("/api/v1/share/status", post(share_status).get(share_status))
        .route("/api/v1/share/stop-all", post(share_stop_all))
        .route("/api/v1/share/{id}", delete(share_stop))
        .route("/share/{id}", get(share_download))
        .route("/api/v1/realtime/has-key", get(realtime_has_key))
        .route("/api/v1/realtime/mint-token", post(realtime_mint_token))
}
