//! Harness home resolution, config persistence, and Pro View defaults.

use crate::CoreError;
use crate::damaged::{DamagedFiles, IntegrityIssue, JsonShape, ReadOutcome};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub fn expand_tilde(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    }
    if let Some(rest) = trimmed.strip_prefix("~/").or_else(|| trimmed.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(trimmed)
}

pub fn default_data_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("MD_DATA_DIR")
        && !explicit.trim().is_empty()
    {
        return expand_tilde(&explicit);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("munder-difflin-rs")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessConfig {
    pub harness_home: Option<String>,
    #[serde(default)]
    pub recent_hives: Vec<String>,
    #[serde(default)]
    pub registered_repos: Vec<String>,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_skin")]
    pub skin: String,
    #[serde(default = "default_true")]
    pub orchestrator_may_spawn: bool,
    #[serde(default)]
    pub onboarding_complete: bool,
    #[serde(default)]
    pub notifications: bool,
    #[serde(default)]
    pub office_theme: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_skin() -> String {
    "professional".to_string()
}

fn default_true() -> bool {
    true
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self {
            harness_home: None,
            recent_hives: Vec::new(),
            registered_repos: Vec::new(),
            theme: default_theme(),
            skin: default_skin(),
            orchestrator_may_spawn: true,
            onboarding_complete: false,
            notifications: false,
            office_theme: None,
            extra: Map::new(),
        }
    }
}

pub struct ConfigStore {
    config_path: PathBuf,
    damaged: DamagedFiles,
    last_good: Option<HarnessConfig>,
}

impl ConfigStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            config_path: data_dir.as_ref().join("config.json"),
            damaged: DamagedFiles::new(),
            last_good: None,
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn read(&mut self) -> HarnessConfig {
        match self.damaged.read(&self.config_path, JsonShape::Object) {
            ReadOutcome::Ok(val) => {
                if let Ok(mut cfg) = serde_json::from_value::<HarnessConfig>(val) {
                    if let Some(ref home) = cfg.harness_home {
                        cfg.harness_home = Some(expand_tilde(home).to_string_lossy().into_owned());
                    }
                    self.last_good = Some(cfg.clone());
                    cfg
                } else {
                    self.last_good.clone().unwrap_or_default()
                }
            }
            ReadOutcome::Missing => {
                let def = HarnessConfig::default();
                self.last_good = Some(def.clone());
                def
            }
            ReadOutcome::Damaged | ReadOutcome::Unreadable => {
                self.last_good.clone().unwrap_or_default()
            }
        }
    }

    pub fn update(&mut self, patch: Value) -> Result<HarnessConfig, CoreError> {
        if self.damaged.is_damaged(&self.config_path, JsonShape::Object) {
            return Err(CoreError::InvalidPath(
                "config.json is damaged; writes paused until repaired".to_string(),
            ));
        }

        let current = self.read();
        let mut merged = serde_json::to_value(current)?;
        if let (Some(base_obj), Some(patch_obj)) = (merged.as_object_mut(), patch.as_object()) {
            for (k, v) in patch_obj {
                // Keep sensitive/desktop-only keys sanitized cleanly
                if k == "agentConnections" || k == "tour" {
                    continue;
                }
                if k == "harnessHome"
                    && let Some(s) = v.as_str()
                {
                    let expanded = expand_tilde(s).to_string_lossy().into_owned();
                    base_obj.insert(k.clone(), Value::String(expanded));
                    continue;
                }
                base_obj.insert(k.clone(), v.clone());
            }
        }

        let next: HarnessConfig = serde_json::from_value(merged)?;
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.config_path.with_extension("json.tmp");
        let serialized = serde_json::to_string_pretty(&next)?;
        fs::write(&tmp, serialized)?;
        fs::rename(&tmp, &self.config_path)?;
        self.damaged.mark_wrote(&self.config_path);
        self.last_good = Some(next.clone());
        Ok(next)
    }

    pub fn ensure_harness_home(&self, raw_path: &str) -> Result<PathBuf, CoreError> {
        if raw_path.trim().is_empty() {
            return Err(CoreError::InvalidPath("empty path".to_string()));
        }
        let expanded = expand_tilde(raw_path);
        fs::create_dir_all(&expanded)?;
        Ok(expanded)
    }

    pub fn integrity_issues(&mut self) -> Vec<IntegrityIssue> {
        let _ = self.damaged.read(&self.config_path, JsonShape::Object);
        self.damaged.list()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrip_and_damage_guard() {
        let dir = std::env::temp_dir().join(format!(
            "md-cfg-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut store = ConfigStore::new(&dir);
        let init = store.read();
        assert_eq!(init.skin, "professional");

        let updated = store
            .update(serde_json::json!({
                "harnessHome": "~/my-hive",
                "onboardingComplete": true
            }))
            .unwrap();
        assert!(updated.onboarding_complete);
        assert!(!updated.harness_home.as_deref().unwrap().starts_with("~/"));

        // Corrupt the file on disk: reads fall back to last_good and writes pause.
        fs::write(store.config_path(), "{not json").unwrap();
        let fallback = store.read();
        assert!(fallback.onboarding_complete);
        assert!(store
            .update(serde_json::json!({ "notifications": true }))
            .is_err());
        assert_eq!(store.integrity_issues().len(), 1);

        fs::remove_dir_all(&dir).ok();
    }
}
