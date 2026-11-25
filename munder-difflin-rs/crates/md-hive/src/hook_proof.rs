//! Hook proof & turn-state recorder (`src/main/hookProof.ts`).

use std::collections::{HashMap, VecDeque};

const PROMPTS_KEPT: usize = 8;
const PROMPT_TAIL_CHARS: usize = 48;

pub fn is_turn_event(event: &str) -> bool {
    matches!(
        event,
        "UserPromptSubmit"
            | "PreToolUse"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "Stop"
            | "StopFailure"
            | "SubagentStop"
            | "PreCompact"
            | "PostCompact"
            | "PreInvocation"
            | "PostInvocation"
    )
}

pub fn asks_person(message: Option<&str>) -> bool {
    let m = message.unwrap_or("").trim();
    if m.is_empty() {
        return false;
    }
    let lower = m.to_lowercase();
    let asks = ["permission", "approve", "approval", "confirm", "needs your"]
        .iter()
        .any(|k| lower.contains(k));
    let quiet = ["waiting for your input", "is idle", "waiting for input"]
        .iter()
        .any(|k| lower.contains(k));
    asks && !quiet
}

pub fn prompt_tail(prompt: &str) -> String {
    let compact: String = prompt.chars().filter(|c| !c.is_whitespace()).collect();
    let total = compact.chars().count();
    compact
        .chars()
        .skip(total.saturating_sub(PROMPT_TAIL_CHARS))
        .collect()
}

#[derive(Default)]
struct AgentHooks {
    last_at: HashMap<String, u64>,
    turn_at: u64,
    prompts: VecDeque<(u64, String)>,
    ask_open_at: u64,
}

#[derive(Default)]
pub struct HookProofRecord {
    by_agent: HashMap<String, AgentHooks>,
}

impl HookProofRecord {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note(
        &mut self,
        agent_id: Option<&str>,
        event: &str,
        prompt: Option<&str>,
        message: Option<&str>,
        tool: Option<&str>,
        at: u64,
    ) {
        let Some(agent_id) = agent_id else {
            return;
        };
        let a = self.by_agent.entry(agent_id.to_string()).or_default();
        a.last_at.insert(event.to_string(), at);
        if is_turn_event(event) {
            a.turn_at = at;
        }
        if (event == "Notification" && asks_person(message))
            || (event == "PreToolUse" && tool == Some("AskUserQuestion"))
        {
            a.ask_open_at = at;
        } else if matches!(
            event,
            "PostToolUse" | "PostToolUseFailure" | "UserPromptSubmit" | "SessionStart"
        ) {
            a.ask_open_at = 0;
        }
        if event == "UserPromptSubmit"
            && let Some(p) = prompt
        {
            a.prompts.push_back((at, prompt_tail(p)));
            while a.prompts.len() > PROMPTS_KEPT {
                a.prompts.pop_front();
            }
        }
    }

    pub fn activity_seen(&self, agent_id: &str, since_ms: u64) -> bool {
        self.by_agent
            .get(agent_id)
            .is_some_and(|a| a.turn_at > 0 && a.turn_at >= since_ms)
    }

    pub fn ask_open(&self, agent_id: &str) -> bool {
        self.by_agent
            .get(agent_id)
            .is_some_and(|a| a.ask_open_at > 0)
    }

    pub fn last_prompt(&self, agent_id: &str) -> Option<(u64, String)> {
        self.by_agent
            .get(agent_id)
            .and_then(|a| a.prompts.back().cloned())
    }
}
