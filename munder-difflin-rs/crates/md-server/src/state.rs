use md_core::config::ConfigStore;
use md_db::PersistStore;
use md_hive::{
    breaker::CircuitBreaker,
    control::ControlRegistry,
    hooks::HookEngine,
    manager::HiveManager,
    roster::RosterStore,
    wake_filing::WakeFiling,
};
use md_integrations::{
    file_share::FileShareRegistry,
    secrets::{IntegrationBroker, SecretStore},
    webhooks::WebhookRegistry,
};
use md_pty::{
    manager::PtyManager,
    submitter::{PtySubmitter, TerminalProbes},
};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndedAgentRecord {
    pub agent_id: String,
    pub exit_code: u32,
    pub signal: Option<String>,
    pub print_mode: bool,
    pub at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerEvent {
    pub channel: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdeState {
    pub unsaved_count: usize,
    pub open_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerRecord {
    pub worker_id: String,
    pub req_id: String,
    pub pty_id: String,
    pub name: String,
    pub job: String,
    pub cwd: String,
    pub base_branch: String,
    pub provider: String,
    pub model: Option<String>,
    pub command: String,
    pub started_at: u64,
    pub token_cap: Option<u64>,
    pub has_slack: bool,
    pub releasing: bool,
    pub worktree: Option<String>,
    pub effort: Option<String>,
    pub requested_by: String,
}

#[derive(Clone)]
pub struct AppState {
    pub port: u16,
    pub data_dir: PathBuf,
    pub config: Arc<Mutex<ConfigStore>>,
    pub db: Arc<Mutex<PersistStore>>,
    pub roster: Arc<Mutex<RosterStore>>,
    pub hive: Arc<Mutex<HiveManager>>,
    pub control: Arc<Mutex<ControlRegistry>>,
    pub breaker: Arc<Mutex<CircuitBreaker>>,
    pub wake_filing: Arc<Mutex<WakeFiling>>,
    pub hooks: HookEngine,
    pub pty: PtyManager,
    pub submitter: PtySubmitter,
    pub probes: TerminalProbes,
    pub ide: Arc<Mutex<IdeState>>,
    pub secrets: Arc<Mutex<SecretStore>>,
    pub broker: Arc<Mutex<IntegrationBroker>>,
    pub webhooks: Arc<Mutex<WebhookRegistry>>,
    pub file_share: Arc<Mutex<FileShareRegistry>>,
    pub workers: Arc<Mutex<Vec<WorkerRecord>>>,
    pub workers_history: Arc<Mutex<Vec<serde_json::Value>>>,
    pub ended_agents: Arc<Mutex<HashMap<String, EndedAgentRecord>>>,
    pub integrations_activity: Arc<Mutex<Vec<serde_json::Value>>>,
    pub events_tx: broadcast::Sender<ServerEvent>,
}

impl AppState {
    pub fn emit(&self, channel: &str, payload: serde_json::Value) {
        let _ = self.events_tx.send(ServerEvent {
            channel: channel.to_string(),
            payload,
        });
    }

    pub fn harness_home(&self) -> Option<PathBuf> {
        let mut store = self.config.lock().unwrap();
        store.read().harness_home.map(PathBuf::from)
    }

    pub fn sync_hive_home(&self) {
        let home = self.harness_home();
        self.hive.lock().unwrap().set_home(home);
    }

    pub fn sync_config_runtime(&self, cfg: &md_core::config::HarnessConfig) {
        self.breaker.lock().unwrap().configure_from_harness(cfg);
        let paused_ids: Vec<String> = match cfg.extra.get("autoDeliveryPausedAgents") {
            Some(serde_json::Value::Object(map)) => map
                .iter()
                .filter_map(|(k, v)| v.as_bool().filter(|&b| b).map(|_| k.clone()))
                .collect(),
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect(),
            _ => Vec::new(),
        };
        self.control
            .lock()
            .unwrap()
            .replace_auto_delivery_pauses(&paused_ids);
    }
}
