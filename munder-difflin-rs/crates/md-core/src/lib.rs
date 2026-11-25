pub mod config;
pub mod damaged;
pub mod floor_box;
pub mod repo_fs;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const APP_VERSION: &str = "0.5.10-rs.0";

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub target: String,
    pub pro_view_compact: bool,
    pub enlarged_floor: bool,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "Munder Difflin Web".to_string(),
            version: APP_VERSION.to_string(),
            target: "rust-web".to_string(),
            pro_view_compact: true,
            enlarged_floor: true,
        }
    }
}
