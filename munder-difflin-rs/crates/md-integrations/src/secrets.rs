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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationRecord {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub auth_header: String,
    pub auth_prefix: String,
    pub secret_ref: String,
}

#[derive(Debug, Clone)]
pub struct BrokerCapability {
    pub worker_id: String,
    pub allowed_ids: HashSet<String>,
    pub allow_secrets_read: bool,
}

#[derive(Default)]
pub struct IntegrationBroker {
    records: HashMap<String, IntegrationRecord>,
    tokens: HashMap<String, BrokerCapability>,
}

impl IntegrationBroker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_record(&mut self, rec: IntegrationRecord) -> Result<(), String> {
        if !is_safe_id(&rec.id) {
            return Err("invalid integration id".to_string());
        }
        if !rec.base_url.starts_with("https://") && !rec.base_url.starts_with("http://127.0.0.1") {
            return Err("integration baseUrl must be https:// or loopback".to_string());
        }
        self.records.insert(rec.id.clone(), rec);
        Ok(())
    }

    pub fn list_records(&self) -> Vec<&IntegrationRecord> {
        let mut list: Vec<_> = self.records.values().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        list
    }

    pub fn get_record(&self, id: &str) -> Option<&IntegrationRecord> {
        self.records.get(id)
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
}
