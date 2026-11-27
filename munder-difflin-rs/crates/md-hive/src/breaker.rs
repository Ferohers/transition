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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BreakerAction {
    None,
    Steer,
    Constrain,
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BreakerConfig {
    pub enabled: bool,
    pub hard_stop: bool,
    pub repeated_tool_limit: usize,
    pub error_storm_limit: usize,
    pub token_velocity_per_min: u64,
    pub cost_cap_usd: Option<f64>,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hard_stop: false,
            repeated_tool_limit: 8,
            error_storm_limit: 5,
            token_velocity_per_min: 60_000,
            cost_cap_usd: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BreakerUsageInput {
    pub agent_id: String,
    pub work_tokens: u64,
    pub output_tokens: u64,
    pub usd: f64,
}

#[derive(Debug, Clone)]
pub struct BreakerDecision {
    pub state: BreakerState,
    pub action: BreakerAction,
    pub changed: bool,
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
    last_output_tokens: Option<u64>,
    last_sample_at: u64,
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
            last_output_tokens: None,
            last_sample_at: 0,
            ts: 0,
        }
    }
}

pub struct CircuitBreaker {
    agents: HashMap<String, AgentBreaker>,
    config: BreakerConfig,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            config: BreakerConfig::default(),
        }
    }

    pub fn config(&self) -> &BreakerConfig {
        &self.config
    }

    pub fn configure_from_harness(&mut self, cfg: &md_core::config::HarnessConfig) {
        let mut next = BreakerConfig {
            cost_cap_usd: cfg.extra.get("costCapUsd").and_then(|v| v.as_f64()),
            ..BreakerConfig::default()
        };
        if let Some(cb) = cfg.extra.get("circuitBreaker").and_then(|v| v.as_object()) {
            if let Some(v) = cb.get("enabled").and_then(|v| v.as_bool()) {
                next.enabled = v;
            }
            if let Some(v) = cb.get("hardStop").and_then(|v| v.as_bool()) {
                next.hard_stop = v;
            }
            if let Some(v) = cb.get("repeatedToolLimit").and_then(|v| v.as_u64()) {
                next.repeated_tool_limit = (v as usize).max(1);
            }
            if let Some(v) = cb.get("errorStormLimit").and_then(|v| v.as_u64()) {
                next.error_storm_limit = (v as usize).max(1);
            }
            if let Some(v) = cb.get("tokenVelocityPerMin").and_then(|v| v.as_u64()) {
                next.token_velocity_per_min = v.max(1);
            }
        }
        self.config = next;
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.agents.remove(agent_id);
    }

    pub fn clear_all(&mut self) {
        self.agents.clear();
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
                level: if self.config.enabled {
                    s.level
                } else {
                    BreakerLevel::Healthy
                },
                reason: if self.config.enabled {
                    s.reason.clone()
                } else {
                    String::new()
                },
                ts: s.ts,
                cause: if self.config.enabled { s.cause } else { None },
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
        if !self.config.enabled {
            return None;
        }
        let key = format!(
            "{}:{}",
            tool_name.unwrap_or(""),
            tool_input.map(|v| v.to_string()).unwrap_or_default()
        );
        let limit = self.config.repeated_tool_limit.max(1);
        let hard_stop = self.config.hard_stop;
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

        if s.repeat_count >= limit {
            let next_level = match s.level {
                BreakerLevel::Healthy => BreakerLevel::Steering,
                BreakerLevel::Steering if s.repeat_count >= limit * 2 => BreakerLevel::Constrained,
                BreakerLevel::Constrained if hard_stop && s.repeat_count >= limit * 3 => {
                    BreakerLevel::Stopped
                }
                other => other,
            };
            if next_level != s.level || s.level == BreakerLevel::Healthy {
                s.level = next_level;
                s.reason = format!("Repeated identical tool call {}x", s.repeat_count);
                s.cause = Some(BreakerCause::RepeatLoop);
                s.ts = now;
                return Some(self.state_for(agent_id));
            }
        }
        None
    }

    pub fn clear_error_streak(&mut self, agent_id: &str) {
        if let Some(s) = self.agents.get_mut(agent_id) {
            s.error_count = 0;
        }
    }

    pub fn record_error(&mut self, agent_id: &str, now: u64) -> Option<BreakerState> {
        if !self.config.enabled {
            return None;
        }
        let limit = self.config.error_storm_limit.max(1);
        let hard_stop = self.config.hard_stop;
        let s = self.agents.entry(agent_id.to_string()).or_default();
        s.error_count += 1;
        if s.error_count >= limit {
            let next_level = match s.level {
                BreakerLevel::Healthy => BreakerLevel::Steering,
                BreakerLevel::Steering if s.error_count >= limit * 2 => BreakerLevel::Constrained,
                BreakerLevel::Constrained if hard_stop && s.error_count >= limit * 3 => {
                    BreakerLevel::Stopped
                }
                other => other,
            };
            if next_level != s.level {
                s.level = next_level;
                s.reason = format!("Error storm ({} consecutive failures)", s.error_count);
                s.cause = Some(BreakerCause::Other);
                s.ts = now;
                return Some(self.state_for(agent_id));
            }
        }
        None
    }

    /// Periodic budget & velocity evaluation matching upstream `CircuitBreaker.tick`.
    pub fn tick_spend_and_velocity(
        &mut self,
        inputs: &[BreakerUsageInput],
        agent_caps: &HashMap<String, u64>,
        now_ms: u64,
    ) -> Vec<BreakerDecision> {
        let mut decisions = Vec::new();
        if !self.config.enabled {
            let ids: Vec<String> = self.agents.keys().cloned().collect();
            for id in ids {
                if let Some(s) = self.agents.get_mut(&id)
                    && s.level != BreakerLevel::Healthy
                {
                    s.level = BreakerLevel::Healthy;
                    s.reason.clear();
                    s.cause = None;
                    s.ts = now_ms;
                    decisions.push(BreakerDecision {
                        state: self.state_for(&id),
                        action: BreakerAction::None,
                        changed: true,
                    });
                }
            }
            return decisions;
        }

        let mut top_spender: Option<(String, f64)> = None;
        let mut total_usd = 0.0f64;
        for inp in inputs {
            total_usd += inp.usd;
            if top_spender.as_ref().is_none_or(|(_, max)| inp.usd > *max) {
                top_spender = Some((inp.agent_id.clone(), inp.usd));
            }
        }
        let cost_cap = self.config.cost_cap_usd.filter(|&c| c > 0.0);
        let over_cost_cap_id = match (cost_cap, top_spender) {
            (Some(cap), Some((id, _))) if total_usd > cap => Some(id),
            _ => None,
        };

        let velocity_limit = self.config.token_velocity_per_min;
        let hard_stop = self.config.hard_stop;

        for inp in inputs {
            let s = self.agents.entry(inp.agent_id.clone()).or_default();
            let mut trip_reason: Option<String> = None;

            if let Some(&cap) = agent_caps.get(&inp.agent_id).filter(|&&c| c > 0)
                && inp.work_tokens > cap
            {
                trip_reason = Some(format!(
                    "token limit: {} work tokens over agent cap of {}",
                    inp.work_tokens, cap
                ));
            } else if over_cost_cap_id.as_deref() == Some(inp.agent_id.as_str())
                && let Some(cap) = cost_cap
            {
                trip_reason = Some(format!(
                    "cost cap: floor total ${total_usd:.2} over ${cap:.2} (top spender ${:.2})",
                    inp.usd
                ));
            } else if now_ms >= s.compacting_until
                && let Some(prev_out) = s.last_output_tokens
                && s.last_sample_at > 0
                && now_ms > s.last_sample_at
                && inp.output_tokens > prev_out
            {
                let d_out = (inp.output_tokens - prev_out) as f64;
                let d_min = (now_ms - s.last_sample_at) as f64 / 60_000.0;
                if d_min > 0.0 {
                    let vel = d_out / d_min;
                    if vel > velocity_limit as f64 {
                        trip_reason = Some(format!(
                            "token velocity {}/min > {velocity_limit}/min",
                            vel.round() as u64
                        ));
                    }
                }
            }

            s.last_output_tokens = Some(inp.output_tokens);
            s.last_sample_at = now_ms;

            let prev_level = s.level;
            let next_level = if let Some(ref r) = trip_reason {
                s.reason = r.clone();
                s.cause = Some(BreakerCause::Spend);
                s.ts = now_ms;
                match s.level {
                    BreakerLevel::Healthy => BreakerLevel::Steering,
                    BreakerLevel::Steering => BreakerLevel::Constrained,
                    BreakerLevel::Constrained if hard_stop => BreakerLevel::Stopped,
                    other => other,
                }
            } else if s.cause == Some(BreakerCause::Spend) && s.level != BreakerLevel::Healthy {
                s.ts = now_ms;
                match s.level {
                    BreakerLevel::Stopped => BreakerLevel::Constrained,
                    BreakerLevel::Constrained => BreakerLevel::Steering,
                    BreakerLevel::Steering => {
                        s.reason.clear();
                        s.cause = None;
                        BreakerLevel::Healthy
                    }
                    BreakerLevel::Healthy => BreakerLevel::Healthy,
                }
            } else {
                s.level
            };
            s.level = next_level;

            let changed = next_level != prev_level;
            let escalated = next_level > prev_level;
            let action = if escalated {
                match next_level {
                    BreakerLevel::Healthy => BreakerAction::None,
                    BreakerLevel::Steering => BreakerAction::Steer,
                    BreakerLevel::Constrained => BreakerAction::Constrain,
                    BreakerLevel::Stopped => BreakerAction::Stop,
                }
            } else {
                BreakerAction::None
            };
            if changed || action != BreakerAction::None {
                decisions.push(BreakerDecision {
                    state: self.state_for(&inp.agent_id),
                    action,
                    changed,
                });
            }
        }
        decisions
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

    #[test]
    fn configures_from_harness_and_trips_cost_velocity_caps() {
        let mut cb = CircuitBreaker::new();
        let cfg: md_core::config::HarnessConfig = serde_json::from_value(serde_json::json!({
            "costCapUsd": 5.0,
            "circuitBreaker": {
                "enabled": true,
                "hardStop": true,
                "repeatedToolLimit": 3,
                "errorStormLimit": 2,
                "tokenVelocityPerMin": 1000
            }
        }))
        .unwrap();
        cb.configure_from_harness(&cfg);
        assert_eq!(cb.config().repeated_tool_limit, 3);
        assert_eq!(cb.config().error_storm_limit, 2);
        assert!(cb.config().hard_stop);

        let mut caps = HashMap::new();
        caps.insert("a1".to_string(), 100);
        let inputs = vec![BreakerUsageInput {
            agent_id: "a1".to_string(),
            work_tokens: 150,
            output_tokens: 150,
            usd: 1.0,
        }];
        let dec = cb.tick_spend_and_velocity(&inputs, &caps, 1000);
        assert_eq!(dec.len(), 1);
        assert_eq!(dec[0].state.level, BreakerLevel::Steering);
        assert_eq!(dec[0].action, BreakerAction::Steer);
    }
}
