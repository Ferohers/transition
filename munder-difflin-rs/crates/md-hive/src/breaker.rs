//! Runaway/cost guardrail state machine (`src/main/breaker.ts`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

const COMPACT_GRACE_MS: u64 = 5 * 60_000;
const POST_COMPACT_GRACE_MS: u64 = 90_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum BreakerLevel {
    Healthy,
    Steering,
    Constrained,
    Stopped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BreakerCause {
    NoProgress,
    RepeatLoop,
    Spend,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakerState {
    pub agent_id: String,
    pub level: BreakerLevel,
    pub reason: String,
    pub ts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<BreakerCause>,
}

struct AgentBreaker {
    level: BreakerLevel,
    reason: String,
    cause: Option<BreakerCause>,
    repeat_key: Option<String>,
    repeat_count: usize,
    error_count: usize,
    compacting_until: u64,
    last_distinct_tool_at: u64,
    last_user_prompt_at: u64,
    ts: u64,
}

impl Default for AgentBreaker {
    fn default() -> Self {
        Self {
            level: BreakerLevel::Healthy,
            reason: String::new(),
            cause: None,
            repeat_key: None,
            repeat_count: 0,
            error_count: 0,
            compacting_until: 0,
            last_distinct_tool_at: 0,
            last_user_prompt_at: 0,
            ts: 0,
        }
    }
}

#[derive(Default)]
pub struct CircuitBreaker {
    agents: HashMap<String, AgentBreaker>,
    repeated_tool_limit: usize,
    error_storm_limit: usize,
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            repeated_tool_limit: 8,
            error_storm_limit: 5,
        }
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    pub fn set_state(&mut self, state: BreakerState) {
        let entry = self.agents.entry(state.agent_id).or_default();
        entry.level = state.level;
        entry.reason = state.reason;
        entry.cause = state.cause;
        entry.ts = state.ts;
    }

    pub fn state_for(&self, agent_id: &str) -> BreakerState {
        match self.agents.get(agent_id) {
            Some(s) => BreakerState {
                agent_id: agent_id.to_string(),
                level: s.level,
                reason: s.reason.clone(),
                ts: s.ts,
                cause: s.cause,
            },
            None => BreakerState {
                agent_id: agent_id.to_string(),
                level: BreakerLevel::Healthy,
                reason: String::new(),
                ts: 0,
                cause: None,
            },
        }
    }

    pub fn snapshot(&self) -> Vec<BreakerState> {
        self.agents
            .keys()
            .map(|id| self.state_for(id))
            .collect()
    }

    pub fn record_tool_use(
        &mut self,
        agent_id: &str,
        tool_name: Option<&str>,
        tool_input: Option<&Value>,
        now: u64,
    ) -> Option<BreakerState> {
        let key = format!(
            "{}:{}",
            tool_name.unwrap_or(""),
            tool_input.map(|v| v.to_string()).unwrap_or_default()
        );
        let limit = self.repeated_tool_limit;
        let s = self.agents.entry(agent_id.to_string()).or_default();
        if s.repeat_key.as_deref() == Some(&key) {
            s.repeat_count += 1;
        } else {
            s.repeat_key = Some(key);
            s.repeat_count = 1;
            s.error_count = 0;
            s.last_distinct_tool_at = now;
            if s.level != BreakerLevel::Healthy && s.cause == Some(BreakerCause::RepeatLoop) {
                s.level = BreakerLevel::Healthy;
                s.reason.clear();
                s.cause = None;
                s.ts = now;
                return Some(self.state_for(agent_id));
            }
            return None;
        }

        if s.repeat_count >= limit && s.level == BreakerLevel::Healthy {
            s.level = BreakerLevel::Steering;
            s.reason = format!("Repeated identical tool call {}x", s.repeat_count);
            s.cause = Some(BreakerCause::RepeatLoop);
            s.ts = now;
            return Some(self.state_for(agent_id));
        }
        None
    }

    pub fn record_error(&mut self, agent_id: &str, now: u64) -> Option<BreakerState> {
        let limit = self.error_storm_limit;
        let s = self.agents.entry(agent_id.to_string()).or_default();
        s.error_count += 1;
        if s.error_count >= limit && s.level == BreakerLevel::Healthy {
            s.level = BreakerLevel::Steering;
            s.reason = format!("Error storm ({} consecutive failures)", s.error_count);
            s.cause = Some(BreakerCause::Other);
            s.ts = now;
            return Some(self.state_for(agent_id));
        }
        None
    }

    pub fn record_user_prompt(&mut self, agent_id: &str, now: u64) {
        self.agents
            .entry(agent_id.to_string())
            .or_default()
            .last_user_prompt_at = now;
    }

    pub fn record_compact_start(&mut self, agent_id: &str, now: u64) {
        self.agents
            .entry(agent_id.to_string())
            .or_default()
            .compacting_until = now + COMPACT_GRACE_MS;
    }

    pub fn record_compact_end(&mut self, agent_id: &str, now: u64) {
        let s = self.agents.entry(agent_id.to_string()).or_default();
        if s.compacting_until > 0 {
            s.compacting_until = now + POST_COMPACT_GRACE_MS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_tool_trips_and_distinct_tool_recovers() {
        let mut cb = CircuitBreaker::new();
        let inp = serde_json::json!({ "cmd": "ls" });
        for i in 0..7 {
            assert!(cb.record_tool_use("a1", Some("Bash"), Some(&inp), i).is_none());
        }
        let tripped = cb
            .record_tool_use("a1", Some("Bash"), Some(&inp), 8)
            .expect("8th repeat should trip");
        assert_eq!(tripped.level, BreakerLevel::Steering);
        assert_eq!(tripped.cause, Some(BreakerCause::RepeatLoop));

        let recovered = cb
            .record_tool_use("a1", Some("Read"), None, 9)
            .expect("distinct tool should recover");
        assert_eq!(recovered.level, BreakerLevel::Healthy);
    }
}
