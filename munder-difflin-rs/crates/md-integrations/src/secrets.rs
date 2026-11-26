//! Encrypted write-only secret store + capability-token IntegrationBroker.
//!
//! Secrets are encrypted at rest with ChaCha20-Poly1305 and a 256-bit local
//! key file (`0600` on Unix). Browser/UI endpoints only see presence/metadata,
//! never decrypted secret values.

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use chacha20poly1305::{
    ChaCha20Poly1305, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng},
};
use md_core::repo_fs::is_safe_id;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretMetadata {
    pub key_ref: String,
    pub kind: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SealedEntry {
    kind: String,
    nonce_b64: String,
    ciphertext_b64: String,
    updated_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SealedDiskFile {
    entries: HashMap<String, SealedEntry>,
}

pub struct SecretStore {
    store_path: PathBuf,
    key_path: PathBuf,
    key: [u8; 32],
    entries: HashMap<String, SealedEntry>,
}

impl SecretStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, String> {
        let dir = data_dir.as_ref();
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let key_path = dir.join("broker-master.key");
        let store_path = dir.join("integration-secrets.json");

        let key = if key_path.is_file() {
            let raw = fs::read_to_string(&key_path).map_err(|e| e.to_string())?;
            let bytes = B64.decode(raw.trim()).map_err(|e| e.to_string())?;
            if bytes.len() != 32 {
                return Err("corrupt broker master key length".to_string());
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            arr
        } else {
            let mut arr = [0u8; 32];
            let k = ChaCha20Poly1305::generate_key(&mut OsRng);
            arr.copy_from_slice(k.as_slice());
            fs::write(&key_path, B64.encode(arr)).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
            }
            arr
        };

        let entries = if store_path.is_file() {
            let raw = fs::read_to_string(&store_path).unwrap_or_default();
            serde_json::from_str::<SealedDiskFile>(&raw)
                .map(|d| d.entries)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        Ok(Self {
            store_path,
            key_path,
            key,
            entries,
        })
    }

    pub fn open_in_memory() -> Self {
        let k = ChaCha20Poly1305::generate_key(&mut OsRng);
        let mut key = [0u8; 32];
        key.copy_from_slice(k.as_slice());
        Self {
            store_path: PathBuf::new(),
            key_path: PathBuf::new(),
            key,
            entries: HashMap::new(),
        }
    }

    fn persist(&self) -> Result<(), String> {
        if self.store_path.as_os_str().is_empty() {
            return Ok(());
        }
        let disk = SealedDiskFile {
            entries: self.entries.clone(),
        };
        let json = serde_json::to_string_pretty(&disk).map_err(|e| e.to_string())?;
        let tmp = self.store_path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.store_path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.store_path, fs::Permissions::from_mode(0o600));
            let _ = fs::set_permissions(&self.key_path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn set_secret(&mut self, key_ref: &str, kind: &str, plaintext: &str) -> Result<(), String> {
        if key_ref.trim().is_empty() || key_ref.len() > 160 {
            return Err("invalid secret keyRef".to_string());
        }
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|e| format!("encrypt failed: {e}"))?;
        let entry = SealedEntry {
            kind: kind.to_string(),
            nonce_b64: B64.encode(nonce.as_slice()),
            ciphertext_b64: B64.encode(ct),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        self.entries.insert(key_ref.to_string(), entry);
        self.persist()
    }

    pub fn delete_secret(&mut self, key_ref: &str) -> Result<bool, String> {
        let removed = self.entries.remove(key_ref).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn has_secret(&self, key_ref: &str) -> bool {
        self.entries.contains_key(key_ref)
    }

    pub fn clear_all(&mut self) {
        self.entries.clear();
        let _ = self.persist();
    }

    /// Safe metadata list for UI — never returns plaintext values.
    pub fn list_metadata(&self) -> Vec<SecretMetadata> {
        let mut out: Vec<SecretMetadata> = self
            .entries
            .iter()
            .map(|(k, v)| SecretMetadata {
                key_ref: k.clone(),
                kind: v.kind.clone(),
                updated_at: v.updated_at.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.key_ref.cmp(&b.key_ref));
        out
    }

    /// Server/broker-internal decryption only.
    pub fn reveal_for_broker(&self, key_ref: &str) -> Option<String> {
        let entry = self.entries.get(key_ref)?;
        let nonce_bytes = B64.decode(&entry.nonce_b64).ok()?;
        let ct = B64.decode(&entry.ciphertext_b64).ok()?;
        if nonce_bytes.len() != 12 {
            return None;
        }
        let cipher = ChaCha20Poly1305::new((&self.key).into());
        let nonce = Nonce::from_slice(&nonce_bytes);
        let pt = cipher.decrypt(nonce, ct.as_ref()).ok()?;
        String::from_utf8(pt).ok()
    }

    /// Redact any known secret value (>= 8 chars) from text before logging or display.
    pub fn redact_text(&self, input: &str) -> String {
        let mut out = input.to_string();
        for key_ref in self.entries.keys() {
            if let Some(val) = self.reveal_for_broker(key_ref)
                && val.len() >= 8
            {
                out = out.replace(&val, "[REDACTED]");
            }
        }
        out
    }

    /// Return custom user-defined secrets (`kind == "custom"`) validated for env injection
    /// without overriding keys already present in `existing`.
    pub fn custom_secret_env(&self, existing: &HashMap<String, String>) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for (name, entry) in &self.entries {
            if entry.kind != "custom"
                || existing.contains_key(name)
                || !is_valid_custom_secret_name(name)
            {
                continue;
            }
            if let Some(val) = self.reveal_for_broker(name) {
                out.insert(name.clone(), val);
            }
        }
        out
    }
}

const RESERVED_SECRET_NAMES: &[&str] = &[
    "PATH",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "PWD",
    "TERM",
    "LANG",
    "LC_ALL",
    "AGENT_ID",
    "AGENT_NAME",
    "AGENT_DIR",
    "HIVE_ROOT",
    "MD_BROKER_URL",
    "MD_BROKER_TOKEN",
    "MD_SERVER_URL",
    "MD_HOOK_URL",
    "MD_HOOK_TOKEN",
    "MEMPALACE_PALACE_PATH",
    "HIVE_AUTO_APPROVE",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_GENERATIVE_AI_API_KEY",
    "OPENROUTER_API_KEY",
    "GROQ_API_KEY",
];

pub fn is_valid_custom_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !RESERVED_SECRET_NAMES.contains(&name)
}

fn default_kind() -> String {
    "custom-rest".to_string()
}

fn default_auth_type() -> String {
    "bearer".to_string()
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationRecord {
    pub id: String,
    #[serde(alias = "name", default)]
    pub label: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    pub base_url: String,
    #[serde(default = "default_auth_type")]
    pub auth_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_path: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct BrokerForwardRequest {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct BrokerForwardResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub fn resolve_upstream_url(
    base_url: &str,
    rel_path: &str,
    query: Option<&str>,
) -> Result<reqwest::Url, String> {
    let base = reqwest::Url::parse(base_url.trim()).map_err(|e| format!("invalid baseUrl: {e}"))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err("baseUrl must be http or https".to_string());
    }
    let trimmed = rel_path.trim_start_matches('/');
    if rel_path.starts_with("//")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.contains('\\')
        || trimmed.split('/').any(|seg| seg == ".." || seg == ".")
    {
        return Err("invalid or out-of-bounds path".to_string());
    }
    let mut joined_str = if trimmed.is_empty() {
        base_url.trim().trim_end_matches('/').to_string()
    } else {
        format!("{}/{}", base_url.trim().trim_end_matches('/'), trimmed)
    };
    if let Some(q) = query.filter(|s| !s.is_empty()) {
        let sep = if joined_str.contains('?') { '&' } else { '?' };
        joined_str.push(sep);
        joined_str.push_str(q);
    }
    let resolved = reqwest::Url::parse(&joined_str).map_err(|e| format!("invalid path: {e}"))?;
    if resolved.origin() != base.origin() {
        return Err("upstream origin mismatch".to_string());
    }
    Ok(resolved)
}

fn apply_auth_to_request(
    client: &reqwest::Client,
    method: reqwest::Method,
    record: &IntegrationRecord,
    mut url: reqwest::Url,
    secret: Option<&str>,
) -> Result<reqwest::RequestBuilder, (u16, String)> {
    if record.auth_type != "none" && secret.unwrap_or("").is_empty() {
        return Err((503, "no secret set".to_string()));
    }
    let sec = secret.unwrap_or("");
    if record.auth_type == "query"
        && let Some(param) = record.auth_query.as_deref().filter(|s| !s.is_empty())
    {
        url.query_pairs_mut().append_pair(param, sec);
    }
    let mut builder = client.request(method, url).header("User-Agent", "munder-difflin");
    match record.auth_type.as_str() {
        "bearer" | "github" => {
            builder = builder.bearer_auth(sec);
        }
        "header" => {
            if let Some(h) = record.auth_header.as_deref().filter(|s| !s.is_empty()) {
                builder = builder.header(h, sec);
            }
        }
        "prefix" => {
            if let Some(h) = record.auth_header.as_deref().filter(|s| !s.is_empty()) {
                let prefix = record.auth_prefix.as_deref().unwrap_or("");
                let val = if prefix.is_empty() {
                    sec.to_string()
                } else {
                    format!("{prefix} {sec}")
                };
                builder = builder.header(h, val);
            }
        }
        "basic" => {
            let user = record.auth_user.as_deref().unwrap_or("");
            builder = builder.basic_auth(user, Some(sec));
        }
        _ => {}
    }
    Ok(builder)
}

#[derive(Debug, Clone)]
pub struct BrokerCapability {
    pub worker_id: String,
    pub allowed_ids: HashSet<String>,
    pub allow_secrets_read: bool,
}

#[derive(Default)]
pub struct IntegrationBroker {
    store_path: PathBuf,
    records: HashMap<String, IntegrationRecord>,
    tokens: HashMap<String, BrokerCapability>,
}

impl IntegrationBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(data_dir: impl AsRef<Path>) -> Self {
        let dir = data_dir.as_ref();
        let _ = fs::create_dir_all(dir);
        let store_path = dir.join("integrations.json");
        let records = if store_path.is_file() {
            let raw = fs::read_to_string(&store_path).unwrap_or_default();
            serde_json::from_str::<HashMap<String, IntegrationRecord>>(&raw).unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self {
            store_path,
            records,
            tokens: HashMap::new(),
        }
    }

    fn persist(&self) -> Result<(), String> {
        if self.store_path.as_os_str().is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.records).map_err(|e| e.to_string())?;
        let tmp = self.store_path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.store_path).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn upsert_record(&mut self, mut rec: IntegrationRecord) -> Result<(), String> {
        if !is_safe_id(&rec.id) {
            return Err("invalid integration id".to_string());
        }
        if !rec.base_url.starts_with("https://")
            && !rec.base_url.starts_with("http://127.0.0.1")
            && !rec.base_url.starts_with("http://localhost")
            && !rec.base_url.starts_with("http://[::1]")
        {
            return Err("integration baseUrl must be https:// or loopback".to_string());
        }
        if rec.label.trim().is_empty() {
            rec.label = rec.id.clone();
        }
        if rec.auth_type != "none" && rec.secret_ref.is_none() {
            rec.secret_ref = Some(format!("int:{}", rec.id));
        }
        self.records.insert(rec.id.clone(), rec);
        self.persist()
    }

    pub fn remove_record(&mut self, id: &str) -> Result<bool, String> {
        let removed = self.records.remove(id).is_some();
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn list_records(&self) -> Vec<&IntegrationRecord> {
        let mut list: Vec<_> = self.records.values().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    pub fn get_record(&self, id: &str) -> Option<&IntegrationRecord> {
        self.records.get(id)
    }

    pub async fn test_connection(record: &IntegrationRecord, secret: Option<&str>) -> serde_json::Value {
        let probe = record.probe_path.as_deref().unwrap_or("");
        let url = match resolve_upstream_url(&record.base_url, probe, None) {
            Ok(u) => u,
            Err(e) => return serde_json::json!({ "ok": false, "error": e }),
        };
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(12))
            .build()
        {
            Ok(c) => c,
            Err(e) => return serde_json::json!({ "ok": false, "error": e.to_string() }),
        };
        let builder = match apply_auth_to_request(&client, reqwest::Method::GET, record, url, secret)
        {
            Ok(b) => b,
            Err((status, msg)) => {
                return serde_json::json!({ "ok": false, "status": status, "error": msg });
            }
        };
        match builder.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let ok = resp.status().is_success();
                serde_json::json!({
                    "ok": ok,
                    "status": status,
                    "error": if ok { serde_json::Value::Null } else { serde_json::Value::String(format!("HTTP {status}")) }
                })
            }
            Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Proxy an API request to an integration upstream, keeping credentials inside the broker.
    pub async fn forward_call(
        record: &IntegrationRecord,
        secret: Option<&str>,
        req: BrokerForwardRequest,
    ) -> Result<BrokerForwardResponse, (u16, String)> {
        if !record.enabled {
            return Err((403, "integration is disabled".to_string()));
        }
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|_| (405, "unsupported method".to_string()))?;
        let url = resolve_upstream_url(&record.base_url, &req.path, req.query.as_deref())
            .map_err(|e| (400, e))?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(25))
            .build()
            .map_err(|e| (500, e.to_string()))?;
        let mut builder = apply_auth_to_request(&client, method, record, url, secret)?;
        for (k, v) in req.headers {
            let lower = k.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "host"
                    | "authorization"
                    | "x-md-broker-token"
                    | "content-length"
                    | "connection"
                    | "transfer-encoding"
                    | "accept-encoding"
            ) {
                continue;
            }
            builder = builder.header(&k, &v);
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body);
        }
        let resp = builder.send().await.map_err(|e| (502, e.to_string()))?;
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(ToString::to_string);
        let body = resp
            .bytes()
            .await
            .map_err(|e| (502, e.to_string()))?
            .to_vec();
        Ok(BrokerForwardResponse {
            status,
            content_type,
            body,
        })
    }

    pub fn grant_capability(
        &mut self,
        worker_id: &str,
        allowed_ids: Vec<String>,
        allow_secrets_read: bool,
    ) -> String {
        let token = format!("mdb_{}", Uuid::new_v4().simple());
        self.tokens.insert(
            token.clone(),
            BrokerCapability {
                worker_id: worker_id.to_string(),
                allowed_ids: allowed_ids.into_iter().collect(),
                allow_secrets_read,
            },
        );
        token
    }

    pub fn validate_token(&self, presented: &str) -> Option<&BrokerCapability> {
        let presented_hash = Sha256::digest(presented.as_bytes());
        let mut matched: Option<&BrokerCapability> = None;
        for (tok, cap) in &self.tokens {
            let h = Sha256::digest(tok.as_bytes());
            if constant_time_eq(&presented_hash, &h) {
                matched = Some(cap);
            }
        }
        matched
    }

    pub fn revoke_token(&mut self, token: &str) -> bool {
        self.tokens.remove(token).is_some()
    }

    pub fn clear_all(&mut self) {
        self.records.clear();
        self.tokens.clear();
        let _ = self.persist();
    }
}

pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_only_secret_store_and_broker_capability_work() {
        let mut store = SecretStore::open_in_memory();
        store
            .set_secret("apikey:openai", "provider", "sk-test-secret-12345")
            .unwrap();
        assert!(store.has_secret("apikey:openai"));
        let meta = store.list_metadata();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].key_ref, "apikey:openai");
        assert_eq!(
            store.reveal_for_broker("apikey:openai").as_deref(),
            Some("sk-test-secret-12345")
        );
        assert_eq!(
            store.redact_text("token is sk-test-secret-12345"),
            "token is [REDACTED]"
        );

        let mut broker = IntegrationBroker::new();
        let tok = broker.grant_capability("michael", vec!["github".into()], true);
        let cap = broker.validate_token(&tok).unwrap();
        assert_eq!(cap.worker_id, "michael");
        assert!(cap.allowed_ids.contains("github"));
    }

    #[test]
    fn custom_secret_env_and_upstream_url_guard_work() {
        let mut store = SecretStore::open_in_memory();
        store.set_secret("MY_CUSTOM_TOKEN", "custom", "val-123456").unwrap();
        store.set_secret("OPENAI_API_KEY", "custom", "reserved-skip").unwrap();
        store.set_secret("bad-name", "custom", "invalid-skip").unwrap();
        store.set_secret("int:gh", "integration", "not-custom").unwrap();
        store.set_secret("ALREADY_SET", "custom", "dont-override").unwrap();

        let mut existing = HashMap::new();
        existing.insert("ALREADY_SET".to_string(), "keep-me".to_string());

        let env = store.custom_secret_env(&existing);
        assert_eq!(env.get("MY_CUSTOM_TOKEN").map(String::as_str), Some("val-123456"));
        assert!(!env.contains_key("OPENAI_API_KEY"));
        assert!(!env.contains_key("bad-name"));
        assert!(!env.contains_key("int:gh"));
        assert!(!env.contains_key("ALREADY_SET"));

        let ok = resolve_upstream_url("https://api.example.com/v1", "items/42", Some("q=1"))
            .unwrap();
        assert_eq!(ok.as_str(), "https://api.example.com/v1/items/42?q=1");
        assert!(resolve_upstream_url("https://api.example.com", "../secret", None).is_err());
        assert!(resolve_upstream_url("https://api.example.com", "//evil.com/x", None).is_err());
        assert!(resolve_upstream_url("https://api.example.com", "https://evil.com/x", None).is_err());
    }
}
