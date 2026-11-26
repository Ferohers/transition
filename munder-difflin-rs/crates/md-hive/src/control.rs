//! Operator control over running agents (`src/main/control.ts`).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

const MAX_PENDING_STEERS: usize = 20;
const MAX_STEER_CHARS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentControlSnapshot {
    pub paused: bool,
    pub halted: bool,
    pub auto_delivery_paused: bool,
    pub gated_tools: Vec<String>,
    pub pending_steers: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDecision {
    pub deny: bool,
    pub reason: Option<String>,
}

#[derive(Default)]
struct AgentControl {
    paused: bool,
    halted: bool,
    auto_delivery_paused: bool,
    gated_tools: HashSet<String>,
    steer_queue: VecDeque<String>,
}

#[derive(Default)]
pub struct ControlRegistry {
    map: HashMap<String, AgentControl>,
}

impl ControlRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn ensure(&mut self, id: &str) -> &mut AgentControl {
        self.map.entry(id.to_string()).or_default()
    }

    pub fn pause(&mut self, id: &str, on: bool) {
        self.ensure(id).paused = on;
    }

    pub fn pause_auto_delivery(&mut self, id: &str, on: bool) {
        self.ensure(id).auto_delivery_paused = on;
    }

    pub fn replace_auto_delivery_pauses(&mut self, paused_ids: &[String]) {
        for c in self.map.values_mut() {
            c.auto_delivery_paused = false;
        }
        for id in paused_ids {
            self.ensure(id).auto_delivery_paused = true;
        }
    }

    pub fn gate_tool(&mut self, id: &str, tool: &str, on: bool) {
        let c = self.ensure(id);
        if on {
            c.gated_tools.insert(tool.to_string());
        } else {
            c.gated_tools.remove(tool);
        }
    }

    pub fn steer(&mut self, id: &str, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let q = &mut self.ensure(id).steer_queue;
        if q.len() >= MAX_PENDING_STEERS {
            q.pop_front();
        }
        let capped: String = trimmed.chars().take(MAX_STEER_CHARS).collect();
        q.push_back(capped);
    }

    pub fn halt(&mut self, id: &str) {
        self.ensure(id).halted = true;
    }

    pub fn unhalt(&mut self, id: &str) {
        if let Some(c) = self.map.get_mut(id) {
            c.halted = false;
        }
    }

    pub fn clear_steers(&mut self, id: &str) {
        if let Some(c) = self.map.get_mut(id) {
            c.steer_queue.clear();
        }
    }

    pub fn clear_all(&mut self) {
        self.map.clear();
    }

    pub fn resume(&mut self, id: &str) {
        let c = self.ensure(id);
        c.paused = false;
        c.halted = false;
    }

    pub fn should_halt(&self, id: &str) -> bool {
        self.map.get(id).is_some_and(|c| c.halted)
    }

    pub fn is_auto_delivery_paused(&self, id: &str) -> bool {
        self.map.get(id).is_some_and(|c| c.auto_delivery_paused)
    }

    pub fn tool_decision(&self, id: &str, tool: &str) -> ToolDecision {
        let Some(c) = self.map.get(id) else {
            return ToolDecision {
                deny: false,
                reason: None,
            };
        };
        if c.paused {
            return ToolDecision {
                deny: true,
                reason: Some("Paused by operator — resume from the floor to continue.".to_string()),
            };
        }
        if !tool.is_empty() && c.gated_tools.contains(tool) {
            return ToolDecision {
                deny: true,
                reason: Some(format!("Tool {tool} is gated by the operator.")),
            };
        }
        ToolDecision {
            deny: false,
            reason: None,
        }
    }

    pub fn return_steer(&mut self, id: &str, text: String) {
        self.ensure(id).steer_queue.push_front(text);
    }

    pub fn take_steer(&mut self, id: &str) -> Option<String> {
        self.map.get_mut(id).and_then(|c| c.steer_queue.pop_front())
    }

    pub fn snapshot(&self, id: &str) -> AgentControlSnapshot {
        match self.map.get(id) {
            Some(c) => {
                let mut gated: Vec<String> = c.gated_tools.iter().cloned().collect();
                gated.sort();
                AgentControlSnapshot {
                    paused: c.paused,
                    halted: c.halted,
                    auto_delivery_paused: c.auto_delivery_paused,
                    gated_tools: gated,
                    pending_steers: c.steer_queue.len(),
                }
            }
            None => AgentControlSnapshot {
                paused: false,
                halted: false,
                auto_delivery_paused: false,
                gated_tools: Vec::new(),
                pending_steers: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_gate_steer_and_halt_work() {
        let mut reg = ControlRegistry::new();
        reg.gate_tool("dwight", "Bash", true);
        assert!(reg.tool_decision("dwight", "Bash").deny);
        assert!(!reg.tool_decision("dwight", "Read").deny);

        reg.pause("dwight", true);
        assert!(reg.tool_decision("dwight", "Read").deny);

        reg.halt("dwight");
        assert!(reg.should_halt("dwight"));
        reg.unhalt("dwight");
        assert!(!reg.should_halt("dwight"));
        assert!(reg.tool_decision("dwight", "Read").deny);

        reg.resume("dwight");
        assert!(!reg.tool_decision("dwight", "Read").deny);

        reg.steer("dwight", "stay focused");
        assert_eq!(reg.take_steer("dwight").as_deref(), Some("stay focused"));
    }
}
