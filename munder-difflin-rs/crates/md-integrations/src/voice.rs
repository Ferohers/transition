//! Realtime Michael ephemeral token mint (using local BYOK OpenAI key).

use crate::secrets::SecretStore;
use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

pub const REALTIME_MODEL: &str = "gpt-realtime-2.1";
const OPENAI_KEY_REF: &str = "apikey:openai";
const CLIENT_SECRETS_URL: &str = "https://api.openai.com/v1/realtime/client_secrets";
const LEGACY_SESSIONS_URL: &str = "https://api.openai.com/v1/realtime/sessions";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MintTokenResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_config: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

pub fn has_openai_key(secrets: &SecretStore) -> bool {
    secrets.has_secret(OPENAI_KEY_REF)
}

pub async fn mint_realtime_token(
    openai_key: Option<String>,
    model: Option<&str>,
) -> MintTokenResult {
    let Some(key) = openai_key else {
        return MintTokenResult {
            ok: false,
            token: None,
            expires_at: None,
            session_config: None,
            error: Some("no OpenAI API key set — add apikey:openai in Settings".to_string()),
            code: Some("no_key".to_string()),
        };
    };

    let chosen_model = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(REALTIME_MODEL)
        .to_string();

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return MintTokenResult {
                ok: false,
                token: None,
                expires_at: None,
                session_config: None,
                error: Some(e.to_string()),
                code: Some("network".to_string()),
            };
        }
    };

    let mut res = client
        .post(CLIENT_SECRETS_URL)
        .bearer_auth(&key)
        .json(&json!({ "session": { "type": "realtime", "model": chosen_model } }))
        .send()
        .await;

    if let Ok(ref r) = res
        && r.status().as_u16() == 404
    {
        res = client
            .post(LEGACY_SESSIONS_URL)
            .bearer_auth(&key)
            .json(&json!({ "model": chosen_model }))
            .send()
            .await;
    }

    let resp = match res {
        Ok(r) => r,
        Err(e) => {
            return MintTokenResult {
                ok: false,
                token: None,
                expires_at: None,
                session_config: None,
                error: Some(e.to_string()),
                code: Some("network".to_string()),
            };
        }
    };

    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let msg = body
            .pointer("/error/message")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("HTTP {status}"));
        return MintTokenResult {
            ok: false,
            token: None,
            expires_at: None,
            session_config: None,
            error: Some(msg),
            code: Some("mint_failed".to_string()),
        };
    }

    let token = body
        .get("value")
        .and_then(|v| v.as_str())
        .or_else(|| body.pointer("/client_secret/value").and_then(|v| v.as_str()))
        .map(ToString::to_string);
    let expires_at = body
        .get("expires_at")
        .and_then(|v| v.as_i64())
        .or_else(|| body.pointer("/client_secret/expires_at").and_then(|v| v.as_i64()));

    match token {
        Some(tok) if !tok.is_empty() => MintTokenResult {
            ok: true,
            token: Some(tok),
            expires_at,
            session_config: Some(json!({ "model": chosen_model })),
            error: None,
            code: None,
        },
        _ => MintTokenResult {
            ok: false,
            token: None,
            expires_at: None,
            session_config: None,
            error: Some("mint returned no ephemeral token".to_string()),
            code: Some("no_token".to_string()),
        },
    }
}
