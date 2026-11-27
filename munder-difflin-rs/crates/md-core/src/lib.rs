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

pub fn host_node_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        "linux" => "linux",
        other => other,
    }
}

pub fn host_node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub target: String,
    pub platform: String,
    pub arch: String,
    pub pro_view_compact: bool,
    pub enlarged_floor: bool,
}

impl Default for AppInfo {
    fn default() -> Self {
        Self {
            name: "Duman Office".to_string(),
            version: APP_VERSION.to_string(),
            target: "rust-web".to_string(),
            platform: host_node_platform().to_string(),
            arch: host_node_arch().to_string(),
            pro_view_compact: true,
            enlarged_floor: true,
        }
    }
}
