//! SQLite persistence store mirroring `PersistStore` (`harness.db`) from Munder Difflin 0.5.10.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

pub const COMMAND_HISTORY_LIMIT: usize = 5000;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryEntry {
    pub id: i64,
    pub agent_id: String,
    pub cwd: Option<String>,
    pub text: String,
    pub ts: i64,
}

pub struct PersistStore {
    conn: Connection,
}

impl PersistStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        Self::init_conn(conn)
    }

    pub fn open_in_memory() -> Result<Self, DbError> {
        let conn = Connection::open_in_memory()?;
        Self::init_conn(conn)
    }

    fn init_conn(conn: Connection) -> Result<Self, DbError> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(
                "
                BEGIN;
                CREATE TABLE IF NOT EXISTS kv (
                    key        TEXT PRIMARY KEY,
                    value      TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS command_history (
                    id       INTEGER PRIMARY KEY AUTOINCREMENT,
                    agent_id TEXT NOT NULL,
                    cwd      TEXT,
                    text     TEXT NOT NULL,
                    ts       INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_ch_agent_ts ON command_history(agent_id, ts DESC);
                PRAGMA user_version = 1;
                COMMIT;
                ",
            )?;
        }

        let store = Self { conn };
        store.scrub_external_services()?;
        store.trim_history()?;
        Ok(store)
    }

    /// Remove any legacy `harnessmd.com` / PostHog / cloud-relay state if an
    /// existing `harness.db` from desktop Munder Difflin is opened locally.
    fn scrub_external_services(&self) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM kv
             WHERE key LIKE 'cloud.%'
                OR key LIKE 'relay.%'
                OR key LIKE 'teams.%'
                OR key LIKE 'license.%'
                OR key LIKE 'licence.%'
                OR key LIKE 'connections.%'
                OR key LIKE 'analytics.%'
                OR key LIKE 'telemetry.%'
                OR key LIKE 'update.%'
                OR lower(value) LIKE '%harnessmd.com%'
                OR lower(value) LIKE '%us.i.posthog.com%'",
            [],
        )?;
        Ok(())
    }

    fn is_disallowed_kv(key: &str, encoded: &str) -> bool {
        let key_lc = key.to_ascii_lowercase();
        let val_lc = encoded.to_ascii_lowercase();
        const PREFIXES: &[&str] = &[
            "cloud.",
            "relay.",
            "teams.",
            "license.",
            "licence.",
            "connections.",
            "analytics.",
            "telemetry.",
            "update.",
        ];
        PREFIXES.iter().any(|p| key_lc.starts_with(p))
            || val_lc.contains("harnessmd.com")
            || val_lc.contains("us.i.posthog.com")
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<serde_json::Value>, DbError> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM kv WHERE key = ?", params![key], |r| r.get(0))
            .optional()?;
        Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_kv(&self, key: &str, value: &serde_json::Value) -> Result<(), DbError> {
        let encoded = serde_json::to_string(value)?;
        if Self::is_disallowed_kv(key, &encoded) {
            return Ok(());
        }
        let now = chrono_now_ms();
        self.conn.execute(
            "INSERT INTO kv (key, value, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, encoded, now],
        )?;
        Ok(())
    }

    pub fn add_history(&self, agent_id: &str, cwd: Option<&str>, text: &str) -> Result<(), DbError> {
        let trimmed = text.trim();
        if trimmed.is_empty() || agent_id.is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO command_history (agent_id, cwd, text, ts) VALUES (?, ?, ?, ?)",
            params![agent_id, cwd, trimmed, chrono_now_ms()],
        )?;
        self.trim_history()?;
        Ok(())
    }

    pub fn list_history(
        &self,
        agent_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CommandHistoryEntry>, DbError> {
        let lim = limit.clamp(1, 500) as i64;
        if let Some(id) = agent_id {
            let mut stmt = self.conn.prepare(
                "SELECT id, agent_id, cwd, text, ts
                 FROM command_history WHERE agent_id = ?
                 ORDER BY id DESC LIMIT ?",
            )?;
            let rows = stmt
                .query_map(params![id, lim], map_history_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, agent_id, cwd, text, ts
                 FROM command_history ORDER BY id DESC LIMIT ?",
            )?;
            let rows = stmt
                .query_map(params![lim], map_history_row)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        }
    }

    pub fn search_history(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CommandHistoryEntry>, DbError> {
        let lim = limit.clamp(1, 500) as i64;
        let pattern = format!("%{query}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, agent_id, cwd, text, ts
             FROM command_history WHERE text LIKE ?
             ORDER BY id DESC LIMIT ?",
        )?;
        let rows = stmt
            .query_map(params![pattern, lim], map_history_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn trim_history(&self) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM command_history
             WHERE id <= (SELECT id FROM command_history ORDER BY id DESC LIMIT 1 OFFSET ?)",
            params![COMMAND_HISTORY_LIMIT as i64],
        )?;
        Ok(())
    }
}

fn map_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandHistoryEntry> {
    Ok(CommandHistoryEntry {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        cwd: row.get(2)?,
        text: row.get(3)?,
        ts: row.get(4)?,
    })
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_and_history_roundtrip() {
        let store = PersistStore::open_in_memory().unwrap();
        store.set_kv("theme", &serde_json::json!("dark")).unwrap();
        assert_eq!(
            store.get_kv("theme").unwrap(),
            Some(serde_json::json!("dark"))
        );

        store.add_history("michael", Some("/tmp"), "build roadmap").unwrap();
        let list = store.list_history(Some("michael"), 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].text, "build roadmap");
        let found = store.search_history("roadmap", 10).unwrap();
        assert_eq!(found.len(), 1);

        // External harnessmd.com / cloud keys are silenced from DB persistence.
        store
            .set_kv("cloud.selectedOffice", &serde_json::json!("https://relay.harnessmd.com"))
            .unwrap();
        assert_eq!(store.get_kv("cloud.selectedOffice").unwrap(), None);
        store
            .set_kv("custom.endpoint", &serde_json::json!("https://app.harnessmd.com/api"))
            .unwrap();
        assert_eq!(store.get_kv("custom.endpoint").unwrap(), None);
    }
}
