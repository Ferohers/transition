//! Hook proof & turn-state recorder (`src/main/hookProof.ts`).

use std::collections::{HashMap, VecDeque};

const PROMPTS_KEPT: usize = 8;
const PROMPT_TAIL_CHARS: usize = 48;
const COMPACT_OVER: &[&str] = &[
    "PostCompact",
    "SessionStart",
    "Stop",
    "StopFailure",
    "UserPromptSubmit",
    "PreToolUse",
];

fn strip_pasted_content_tags(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut pos = 0;
    while pos < bytes.len() {
        let Some(rel_lt) = input[pos..].find('<') else {
            out.push_str(&input[pos..]);
            break;
        };
        let lt = pos + rel_lt;
        out.push_str(&input[pos..lt]);
        let rest = &input[lt..];
        let is_tag = rest
            .get(..15)
            .is_some_and(|s| s.eq_ignore_ascii_case("<pasted_content"))
            || rest
                .get(..16)
                .is_some_and(|s| s.eq_ignore_ascii_case("</pasted_content"));
        if is_tag && let Some(gt_rel) = rest.find('>') {
            pos = lt + gt_rel + 1;
        } else {
            out.push('<');
            pos = lt + 1;
        }
    }
    out
}

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
    let cleaned = strip_pasted_content_tags(prompt);
    let compact: String = cleaned.chars().filter(|c| !c.is_whitespace()).collect();
    let total = compact.chars().count();
    compact
        .chars()
        .skip(total.saturating_sub(PROMPT_TAIL_CHARS))
        .collect()
}

pub fn tail_matches(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a == b || a.ends_with(b) || b.ends_with(a))
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

    pub fn event_seen(&self, agent_id: &str, since_ms: u64, events: &[&str]) -> bool {
        let Some(a) = self.by_agent.get(agent_id) else {
            return false;
        };
        events
            .iter()
            .any(|e| a.last_at.get(*e).copied().is_some_and(|at| at > 0 && at >= since_ms))
    }

    pub fn any_seen(&self, agent_id: &str, since_ms: u64) -> bool {
        self.by_agent
            .get(agent_id)
            .is_some_and(|a| a.last_at.values().any(|&at| at > 0 && at >= since_ms))
    }

    pub fn ask_open(&self, agent_id: &str) -> bool {
        self.by_agent
            .get(agent_id)
            .is_some_and(|a| a.ask_open_at > 0)
    }

    pub fn compact_running_since(
        &self,
        agent_id: &str,
        since_ms: u64,
        max_ms: u64,
        now_ms: u64,
    ) -> u64 {
        let Some(a) = self.by_agent.get(agent_id) else {
            return 0;
        };
        let pre = a.last_at.get("PreCompact").copied().unwrap_or(0);
        if pre == 0 || pre < since_ms || now_ms.saturating_sub(pre) >= max_ms {
            return 0;
        }
        if COMPACT_OVER
            .iter()
            .any(|e| a.last_at.get(*e).copied().unwrap_or(0) > pre)
        {
            0
        } else {
            pre
        }
    }

    pub fn last_event_at(&self, agent_id: &str, event: &str) -> u64 {
        self.by_agent
            .get(agent_id)
            .and_then(|a| a.last_at.get(event).copied())
            .unwrap_or(0)
    }

    pub fn last_prompt(&self, agent_id: &str) -> Option<(u64, String)> {
        self.by_agent
            .get(agent_id)
            .and_then(|a| a.prompts.back().cloned())
    }

    pub fn prompt_seen(&self, agent_id: &str, since_ms: u64, tail: &str) -> bool {
        self.by_agent.get(agent_id).is_some_and(|a| {
            a.prompts
                .iter()
                .any(|(at, pt)| *at >= since_ms && tail_matches(pt, tail))
        })
    }

    pub fn recent_prompts(&self, agent_id: &str) -> Vec<(u64, String)> {
        self.by_agent
            .get(agent_id)
            .map(|a| a.prompts.iter().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_tail_strips_pasted_content_tags_and_whitespace() {
        let raw = "hello <pasted_content name=\"a.txt\">world\nfoo</pasted_content>";
        assert_eq!(prompt_tail(raw), "helloworldfoo");
    }

    #[test]
    fn prompt_seen_and_compact_running_since_track_lifecycle() {
        let mut r = HookProofRecord::new();
        r.note(Some("a1"), "PreCompact", None, None, None, 1000);
        assert_eq!(r.compact_running_since("a1", 500, 10_000, 2000), 1000);
        r.note(Some("a1"), "PostCompact", None, None, None, 2500);
        assert_eq!(r.compact_running_since("a1", 500, 10_000, 3000), 0);
        r.note(
            Some("a1"),
            "UserPromptSubmit",
            Some("please run verify"),
            None,
            None,
            4000,
        );
        assert!(r.prompt_seen("a1", 4000, &prompt_tail("please run verify")));
        assert!(!r.prompt_seen("a1", 4001, &prompt_tail("please run verify")));
        assert!(r.event_seen("a1", 4000, &["UserPromptSubmit"]));
        assert_eq!(r.last_event_at("a1", "UserPromptSubmit"), 4000);
    }
}
