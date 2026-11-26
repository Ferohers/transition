//! Generic secret-gated inbound webhooks and TriggerHistory ledger.
//!
//! Secrets are compared in constant time (with a decoy compare on unknown
//! endpoints to prevent enumeration) and are never persisted into history.

use crate::secrets::constant_time_eq;
use md_core::repo_fs::is_safe_id;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub const TRIGGER_HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerMode {
    AutoDispatch,
    AwaitingApproval,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookEndpoint {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub secret: String,
    pub mode: TriggerMode,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookEndpointInput {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub secret: String,
    #[serde(default = "default_mode")]
    pub mode: TriggerMode,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn default_mode() -> TriggerMode {
    TriggerMode::AwaitingApproval
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookInboundPayload {
    pub message: String,
    pub title: Option<String>,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerHistoryEntry {
    pub id: String,
    pub source: String,
    pub source_id: String,
    pub source_name: String,
    pub direction: String,
    pub peer: Option<String>,
    pub title: Option<String>,
    pub body: String,
    pub decision: String,
    pub task_id: Option<String>,
    pub token: String,
    pub at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredWebhookEndpoint {
    pub id: String,
    pub name: String,
    pub secret: String,
    pub mode: TriggerMode,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

pub struct WebhookRegistry {
    endpoints: HashMap<String, WebhookEndpoint>,
    decoy_secret: String,
    endpoints_path: Option<PathBuf>,
    history_path: Option<PathBuf>,
    history: Vec<TriggerHistoryEntry>,
}

impl WebhookRegistry {
    pub fn open(data_dir: Option<&Path>) -> Self {
        let endpoints_path = data_dir.map(|d| d.join("webhook-endpoints.json"));
        let endpoints = endpoints_path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<Vec<StoredWebhookEndpoint>>(&raw).ok())
            .map(|list| {
                list.into_iter()
                    .map(|s| {
                        (
                            s.id.clone(),
                            WebhookEndpoint {
                                id: s.id,
                                name: s.name,
                                secret: s.secret,
                                mode: s.mode,
                                extra: s.extra,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let history_path = data_dir.map(|d| d.join("trigger-history.json"));
        let history = history_path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<Vec<TriggerHistoryEntry>>(&raw).ok())
            .unwrap_or_default();
        Self {
            endpoints,
            decoy_secret: Uuid::new_v4().simple().to_string(),
            endpoints_path,
            history_path,
            history,
        }
    }

    pub fn upsert_endpoint(&mut self, input: WebhookEndpointInput) -> Result<WebhookEndpoint, String> {
        let secret = if input.secret.trim().is_empty() {
            self.endpoints
                .get(&input.id)
                .map(|e| e.secret.clone())
                .unwrap_or_default()
        } else {
            input.secret
        };
        if !is_safe_id(&input.id) || secret.trim().is_empty() {
            return Err("invalid webhook endpoint id or empty secret".to_string());
        }
        let name = if input.name.trim().is_empty() {
            input.id.clone()
        } else {
            input.name
        };
        let ep = WebhookEndpoint {
            id: input.id.clone(),
            name,
            secret,
            mode: input.mode,
            extra: input.extra,
        };
        self.endpoints.insert(input.id, ep.clone());
        self.persist_endpoints();
        Ok(ep)
    }

    pub fn replace_endpoints(&mut self, incoming: Vec<WebhookEndpointInput>) -> Vec<WebhookEndpoint> {
        let mut next = HashMap::new();
        for input in incoming {
            if !is_safe_id(&input.id) || next.contains_key(&input.id) {
                continue;
            }
            let secret = if input.secret.trim().is_empty() {
                self.endpoints
                    .get(&input.id)
                    .map(|e| e.secret.clone())
                    .unwrap_or_default()
            } else {
                input.secret
            };
            if secret.trim().is_empty() {
                continue;
            }
            let name = if input.name.trim().is_empty() {
                input.id.clone()
            } else {
                input.name
            };
            next.insert(
                input.id.clone(),
                WebhookEndpoint {
                    id: input.id,
                    name,
                    secret,
                    mode: input.mode,
                    extra: input.extra,
                },
            );
        }
        self.endpoints = next;
        self.persist_endpoints();
        self.list_endpoints()
    }

    pub fn list_endpoints(&self) -> Vec<WebhookEndpoint> {
        let mut out: Vec<_> = self.endpoints.values().cloned().collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn delete_endpoint(&mut self, id: &str) -> bool {
        let removed = self.endpoints.remove(id).is_some();
        if removed {
            self.persist_endpoints();
        }
        removed
    }

    pub fn clear_all(&mut self) {
        self.endpoints.clear();
        self.history.clear();
        self.persist_endpoints();
        self.persist_history();
    }

    fn persist_endpoints(&self) {
        let Some(ref p) = self.endpoints_path else {
            return;
        };
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let mut rows: Vec<StoredWebhookEndpoint> = self
            .endpoints
            .values()
            .map(|e| StoredWebhookEndpoint {
                id: e.id.clone(),
                name: e.name.clone(),
                secret: e.secret.clone(),
                mode: e.mode.clone(),
                extra: e.extra.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.id.cmp(&b.id));
        if let Ok(json) = serde_json::to_string_pretty(&rows) {
            let _ = fs::write(p, json);
        }
    }

    pub fn accept_inbound(
        &mut self,
        endpoint_id: &str,
        presented_secret: &str,
        payload: WebhookInboundPayload,
    ) -> Result<TriggerHistoryEntry, &'static str> {
        let ep = self.endpoints.get(endpoint_id);
        let enabled = ep
            .and_then(|e| e.extra.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let expected = ep.map(|e| e.secret.as_bytes()).unwrap_or(self.decoy_secret.as_bytes());
        let ok = constant_time_eq(presented_secret.as_bytes(), expected) && ep.is_some() && enabled;
        if !ok {
            return Err("unauthorized");
        }
        let ep = ep.unwrap();
        let token = Uuid::new_v4().simple().to_string();
        let decision = match ep.mode {
            TriggerMode::AutoDispatch => "approved",
            TriggerMode::AwaitingApproval => "pending",
        };
        let entry = TriggerHistoryEntry {
            id: Uuid::new_v4().simple().to_string()[..16].to_string(),
            source: "webhook".to_string(),
            source_id: ep.id.clone(),
            source_name: ep.name.clone(),
            direction: "inbound".to_string(),
            peer: payload.from,
            title: payload.title,
            body: payload.message,
            decision: decision.to_string(),
            task_id: None,
            token,
            at: chrono::Utc::now().timestamp_millis(),
        };
        self.history.insert(0, entry.clone());
        self.history.truncate(TRIGGER_HISTORY_LIMIT);
        self.persist_history();
        Ok(entry)
    }

    pub fn lookup_by_token(&self, endpoint_id: &str, token: &str) -> Option<&TriggerHistoryEntry> {
        self.history
            .iter()
            .find(|e| e.source_id == endpoint_id && constant_time_eq(e.token.as_bytes(), token.as_bytes()))
    }

    pub fn list_history(&self) -> &[TriggerHistoryEntry] {
        &self.history
    }

    pub fn decide(&mut self, entry_id: &str, decision: &str, task_id: Option<String>) -> Option<TriggerHistoryEntry> {
        let entry = self.history.iter_mut().find(|e| e.id == entry_id)?;
        entry.decision = decision.to_string();
        if task_id.is_some() {
            entry.task_id = task_id;
        }
        let cloned = entry.clone();
        self.persist_history();
        Some(cloned)
    }

    fn persist_history(&self) {
        let Some(ref p) = self.history_path else {
            return;
        };
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.history) {
            let _ = fs::write(p, json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_constant_time_auth_and_approval_ledger_work() {
        let mut reg = WebhookRegistry::open(None);
        reg.upsert_endpoint(WebhookEndpointInput {
            id: "ci-hook".into(),
            name: "CI".into(),
            secret: "super-secret".into(),
            mode: TriggerMode::AwaitingApproval,
            extra: Default::default(),
        })
        .unwrap();

        assert!(
            reg.accept_inbound(
                "ci-hook",
                "wrong",
                WebhookInboundPayload {
                    message: "deploy".into(),
                    title: None,
                    from: None,
                },
            )
            .is_err()
        );

        let entry = reg
            .accept_inbound(
                "ci-hook",
                "super-secret",
                WebhookInboundPayload {
                    message: "deploy staging".into(),
                    title: Some("Deploy".into()),
                    from: Some("ci".into()),
                },
            )
            .unwrap();
        assert_eq!(entry.decision, "pending");
        assert!(reg.lookup_by_token("ci-hook", &entry.token).is_some());
        let approved = reg.decide(&entry.id, "approved", Some("MD-20".into())).unwrap();
        assert_eq!(approved.task_id.as_deref(), Some("MD-20"));
    }
}
