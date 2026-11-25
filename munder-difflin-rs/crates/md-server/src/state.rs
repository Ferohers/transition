use md_core::config::ConfigStore;
use md_db::PersistStore;
use md_hive::{
    breaker::CircuitBreaker,
    control::ControlRegistry,
    hooks::HookEngine,
    manager::HiveManager,
    roster::RosterStore,
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

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

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Mutex<ConfigStore>>,
    pub db: Arc<Mutex<PersistStore>>,
    pub roster: Arc<Mutex<RosterStore>>,
    pub hive: Arc<Mutex<HiveManager>>,
    pub control: Arc<Mutex<ControlRegistry>>,
    pub breaker: Arc<Mutex<CircuitBreaker>>,
    pub hooks: HookEngine,
    pub pty: PtyManager,
    pub submitter: PtySubmitter,
    pub probes: TerminalProbes,
    pub ide: Arc<Mutex<IdeState>>,
    pub secrets: Arc<Mutex<SecretStore>>,
    pub broker: Arc<Mutex<IntegrationBroker>>,
    pub webhooks: Arc<Mutex<WebhookRegistry>>,
    pub file_share: Arc<Mutex<FileShareRegistry>>,
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
}
