//! Silenced stubs for upstream features that depend on `harnessmd.com`,
//! `nango.harnessmd.com`, `relay.harnessmd.com`, updater checks, or analytics.
//!
//! Returning deterministic offline/disabled JSON responses keeps UI callers
//! clean without ever initiating external network calls or license checks.

use crate::state::AppState;
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};

async fn silenced_list() -> Json<Vec<Value>> {
    Json(Vec::new())
}

async fn silenced_disabled() -> Json<Value> {
    Json(json!({
        "ok": false,
        "silenced": true,
        "localOnly": true,
        "error": "disabled",
        "detail": "External harnessmd.com service is disabled in local-only mode"
    }))
}

async fn silenced_ok_noop() -> Json<Value> {
    Json(json!({
        "ok": true,
        "silenced": true,
        "localOnly": true
    }))
}

async fn silenced_cloud_selected() -> Json<Value> {
    Json(Value::Null)
}

async fn silenced_sandbox_status() -> Json<Value> {
    Json(json!({
        "installed": false,
        "running": false,
        "silenced": true,
        "localOnly": true
    }))
}

async fn silenced_update_status() -> Json<Value> {
    Json(json!({
        "state": "disabled",
        "localOnly": true,
        "silenced": true
    }))
}

async fn silenced_license_status() -> Json<Value> {
    Json(json!({
        "ok": true,
        "localOnly": true,
        "silenced": true,
        "plan": "local",
        "state": "active"
    }))
}

async fn silenced_pricing() -> Json<Value> {
    Json(Value::Null)
}

pub fn silenced_router() -> Router<AppState> {
    Router::new()
        // Cloud Offices / Relay (`relay.harnessmd.com`)
        .route("/api/v1/cloud/list", get(silenced_list))
        .route("/api/v1/cloud/selected", get(silenced_cloud_selected))
        .route("/api/v1/cloud/{*rest}", post(silenced_disabled).get(silenced_disabled))
        // Cloud Sandbox
        .route("/api/v1/sandbox/status", get(silenced_sandbox_status))
        .route("/api/v1/sandbox/{*rest}", post(silenced_disabled).get(silenced_disabled))
        // Teams (`app.harnessmd.com/desktop/signin`, `/team/*`)
        .route("/api/v1/teams/{*rest}", post(silenced_disabled).get(silenced_disabled))
        // Managed OAuth Connections (`nango.harnessmd.com` + `harnessmd.com`)
        .route("/api/v1/connections/list", get(silenced_list))
        .route("/api/v1/connections/grants", get(silenced_list))
        .route("/api/v1/connections/{*rest}", post(silenced_disabled).get(silenced_disabled))
        // License / Billing / Pricing (`harnessmd.com/api/pricing`, `/pro/checkout`)
        .route("/api/v1/license/status", get(silenced_license_status))
        .route("/api/v1/license/{*rest}", post(silenced_disabled))
        .route("/api/v1/pricing", get(silenced_pricing))
        // Updater (`app.harnessmd.com/releases`, `api.github.com/repos/HarnessMD/...`)
        .route("/api/v1/update/status", get(silenced_update_status))
        .route("/api/v1/update/check", post(silenced_update_status))
        .route("/api/v1/update/{*rest}", post(silenced_ok_noop))
        // Outbound product analytics (`us.i.posthog.com`)
        .route("/api/v1/analytics/{*rest}", post(silenced_ok_noop))
        // Voice dictation / Puck (omitted per user request)
        .route("/api/v1/freeflow/{*rest}", post(silenced_disabled).get(silenced_disabled))
        .route("/api/v1/puck/{*rest}", post(silenced_disabled).get(silenced_disabled))
}
