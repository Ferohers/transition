//! Local-only HTTP file sharing (`fileShare:*`) without external tunnels.

use crate::secrets::constant_time_eq;
use md_core::repo_fs::safe_resolve;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFileEntry {
    pub id: String,
    pub file_name: String,
    #[serde(skip_serializing)]
    pub abs_path: PathBuf,
    pub bytes: u64,
    pub password_protected: bool,
    #[serde(skip_serializing)]
    password_hash: Option<[u8; 32]>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareCreateReq {
    pub root: String,
    pub rel: String,
    pub password: Option<String>,
}

#[derive(Default)]
pub struct FileShareRegistry {
    shares: HashMap<String, SharedFileEntry>,
}

impl FileShareRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_share(&mut self, req: &ShareCreateReq) -> Result<SharedFileEntry, String> {
        let abs = safe_resolve(&req.root, &req.rel).ok_or_else(|| "path escapes root".to_string())?;
        let meta = std::fs::metadata(&abs).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("only regular files can be shared".to_string());
        }
        let file_name = abs
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string());
        let id = Uuid::new_v4().simple().to_string()[..16].to_string();
        let password_hash = req
            .password
            .as_deref()
            .filter(|p| !p.is_empty())
            .map(|p| {
                let mut h = [0u8; 32];
                h.copy_from_slice(&Sha256::digest(p.as_bytes()));
                h
            });
        let entry = SharedFileEntry {
            id: id.clone(),
            file_name,
            abs_path: abs,
            bytes: meta.len(),
            password_protected: password_hash.is_some(),
            password_hash,
            created_at: chrono::Utc::now().timestamp_millis(),
        };
        self.shares.insert(id, entry.clone());
        Ok(entry)
    }

    pub fn list_shares(&self) -> Vec<SharedFileEntry> {
        let mut out: Vec<_> = self.shares.values().cloned().collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        out
    }

    pub fn stop_share(&mut self, id: &str) -> bool {
        self.shares.remove(id).is_some()
    }

    pub fn stop_all(&mut self) -> usize {
        let n = self.shares.len();
        self.shares.clear();
        n
    }

    pub fn get_share(&self, id: &str) -> Option<&SharedFileEntry> {
        self.shares.get(id)
    }

    pub fn authorize(&self, id: &str, password: Option<&str>) -> Result<&SharedFileEntry, &'static str> {
        let entry = self.shares.get(id).ok_or("not_found")?;
        if let Some(expected) = entry.password_hash {
            let provided = password.unwrap_or("");
            let got = Sha256::digest(provided.as_bytes());
            if !constant_time_eq(&expected, &got) {
                return Err("unauthorized");
            }
        }
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_file_with_optional_password() {
        let dir = std::env::temp_dir().join(format!(
            "md-share-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("note.txt"), "hello").unwrap();

        let mut reg = FileShareRegistry::new();
        let entry = reg
            .create_share(&ShareCreateReq {
                root: dir.to_string_lossy().into_owned(),
                rel: "note.txt".into(),
                password: Some("pass123".into()),
            })
            .unwrap();
        assert!(reg.authorize(&entry.id, Some("wrong")).is_err());
        assert!(reg.authorize(&entry.id, Some("pass123")).is_ok());
        assert!(reg.stop_share(&entry.id));
        std::fs::remove_dir_all(&dir).ok();
    }
}
