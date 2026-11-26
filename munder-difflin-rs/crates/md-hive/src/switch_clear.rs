//! Switch-of-work auto-clear & Codex rollout size auto-clear
//! (`src/shared/switchClear.ts`, `src/shared/codexSizeClear.ts`).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::wake_filing::{is_wake_worthy, wake_options_for_provider};

pub const DEFAULT_MIN_WAIT_MS: u64 = 10 * 60_000;
pub const DEFAULT_COOLDOWN_MS: u64 = 10 * 60_000;
pub const HANDOFF_TIMEOUT_MS: u64 = 3 * 60_000;
pub const BACKGROUND_SUBAGENT_TTL_MS: u64 = 30 * 60_000;

pub const CODEX_ROLLOUT_CLEAR_BYTES: u64 = 100 * 1024 * 1024;
pub const CODEX_SIZE_CHECK_MS: u64 = 60_000;

const HANDOFF_OPEN: &str = "This session is being cleared before starting other work.";
const SIZE_HANDOFF_OPEN: &str = "This Codex session log has grown past 100 MB, so it will be restarted now.";
const ASKED_HANDOFF_OPEN: &str = "The orchestrator asked to clear this session before continuing.";

const HANDOFF_TAIL: &str = "Write the key state you would want a fresh session to keep (decisions taken, files edited or read, commands that passed, open questions) in one short markdown note, under 2 KB, to {{HANDOFF}}. Then say \"ready\". Do not start the new work yet.";
const SIZE_HANDOFF_TAIL: &str = "Write the key state you would want a fresh session to keep (work in progress, decisions taken, files edited or read, commands that passed, open questions) in one short markdown note, under 2 KB, to {{HANDOFF}}. Then say \"ready\". Keep your current work untouched so you can resume it right after.";

const SWITCH_CLEARED_LINE: &str = "New session after clearing the previous one; your handoff note is at {{HANDOFF}} — read it if you need context.";
const PREVIOUS_HANDOFF_LINE: &str = "You wrote an earlier handoff note at {{HANDOFF}}; it may still hold background from before the clear before that.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchClearRule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_min_wait_ms")]
    pub min_wait_ms: u64,
    #[serde(default = "default_cooldown_ms")]
    pub cooldown_ms: u64,
}

fn default_min_wait_ms() -> u64 {
    DEFAULT_MIN_WAIT_MS
}

fn default_cooldown_ms() -> u64 {
    DEFAULT_COOLDOWN_MS
}

impl Default for SwitchClearRule {
    fn default() -> Self {
        Self {
            enabled: false,
            min_wait_ms: DEFAULT_MIN_WAIT_MS,
            cooldown_ms: DEFAULT_COOLDOWN_MS,
        }
    }
}

impl SwitchClearRule {
    pub fn from_json(raw: Option<&Value>) -> Self {
        let Some(obj) = raw.and_then(|v| v.as_object()) else {
            return Self::default();
        };
        let enabled = obj.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
        let min_wait_ms = obj
            .get("minWaitMs")
            .and_then(|v| v.as_u64())
            .filter(|&v| v > 0)
            .unwrap_or(DEFAULT_MIN_WAIT_MS);
        let cooldown_ms = obj
            .get("cooldownMs")
            .and_then(|v| v.as_u64())
            .filter(|&v| v > 0)
            .unwrap_or(DEFAULT_COOLDOWN_MS);
        Self {
            enabled,
            min_wait_ms,
            cooldown_ms,
        }
    }
}

pub fn clear_command_for_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" | "codex" | "kimi" | "antigravity" | "gemini" | "qwen" => Some("/clear"),
        "grok" | "opencode" | "pi" => Some("/new"),
        _ => None,
    }
}

pub fn switch_clear_command(provider: &str) -> Option<&'static str> {
    if wake_options_for_provider(provider).self_file {
        None
    } else {
        clear_command_for_provider(provider)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypedKind {
    Handoff,
    Clear,
    Wake,
    Other,
}

pub fn typed_kind(text: &str, clear_cmd: Option<&str>) -> TypedKind {
    let t = text.trim();
    if let Some(cmd) = clear_cmd
        && t == cmd
    {
        return TypedKind::Clear;
    }
    if t.starts_with(HANDOFF_OPEN)
        || t.starts_with(SIZE_HANDOFF_OPEN)
        || t.starts_with(ASKED_HANDOFF_OPEN)
    {
        return TypedKind::Handoff;
    }
    if t.starts_with("You have ") || t.contains("hive inbox message") {
        return TypedKind::Wake;
    }
    TypedKind::Other
}

fn work_line(work: Option<&str>) -> String {
    match work.map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => format!(" Work context: {w}."),
        None => String::new(),
    }
}

pub fn handoff_request_text(work: Option<&str>, handoff_path: &str) -> String {
    format!(
        "{HANDOFF_OPEN}{} {}",
        work_line(work),
        HANDOFF_TAIL.replace("{{HANDOFF}}", handoff_path)
    )
}

pub fn size_handoff_request_text(work: Option<&str>, handoff_path: &str) -> String {
    format!(
        "{SIZE_HANDOFF_OPEN}{} {}",
        work_line(work),
        SIZE_HANDOFF_TAIL.replace("{{HANDOFF}}", handoff_path)
    )
}

pub fn asked_handoff_request_text(work: Option<&str>, handoff_path: &str) -> String {
    format!(
        "{ASKED_HANDOFF_OPEN}{} {}",
        work_line(work),
        SIZE_HANDOFF_TAIL.replace("{{HANDOFF}}", handoff_path)
    )
}

pub fn with_cleared_lead(
    nudge: &str,
    handoff_path: &str,
    one_line: bool,
    prior_session_kept: bool,
) -> String {
    let lead = SWITCH_CLEARED_LINE.replace("{{HANDOFF}}", handoff_path);
    let extra = if prior_session_kept {
        Some(PREVIOUS_HANDOFF_LINE.replace("{{HANDOFF}}", handoff_path))
    } else {
        None
    };
    if one_line {
        match extra {
            Some(ex) => format!("{lead} {ex} {nudge}"),
            None => format!("{lead} {nudge}"),
        }
    } else {
        match extra {
            Some(ex) => format!("{lead}\n{ex}\n\n{nudge}"),
            None => format!("{lead}\n\n{nudge}"),
        }
    }
}

pub fn oldest_waking_work(mail: &[Value]) -> Option<String> {
    let mut best: Option<(i64, String)> = None;
    for msg in mail {
        if !is_wake_worthy(msg) {
            continue;
        }
        let work = msg
            .get("work")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|w| !w.is_empty())
            .map(ToString::to_string);
        let Some(work) = work else {
            continue;
        };
        let ts = msg
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(parse_iso_ms)
            .unwrap_or(i64::MAX);
        match &best {
            Some((best_ts, _)) if ts >= *best_ts => {}
            _ => best = Some((ts, work)),
        }
    }
    best.map(|(_, w)| w)
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    // Handles RFC3339 timestamps if present; falls back lexicographically when equal precision isn't needed.
    // Using simple parser avoids adding extra crates.
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    let hh: i64 = s.get(11..13)?.parse().ok()?;
    let mm: i64 = s.get(14..16)?.parse().ok()?;
    let ss: i64 = s.get(17..19)?.parse().ok()?;
    Some(((((y * 366 + m * 31 + d) * 24 + hh) * 60 + mm) * 60 + ss) * 1000)
}

#[derive(Debug, Clone)]
pub struct HookFacts {
    pub agent_id: String,
    pub event: String,
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub tool_use_id: Option<String>,
    pub subagent_id: Option<String>,
    pub at_ms: u64,
}

#[derive(Default)]
pub struct BackgroundWork {
    by_agent: HashMap<String, HashMap<String, u64>>,
}

impl BackgroundWork {
    pub fn note_hook(&mut self, facts: &HookFacts) {
        let entry = self.by_agent.entry(facts.agent_id.clone()).or_default();
        match facts.event.as_str() {
            "SessionStart" => {
                entry.clear();
            }
            "PreToolUse" => {
                if matches!(facts.tool.as_deref(), Some("Task" | "Agent")) {
                    let key = facts
                        .tool_use_id
                        .as_deref()
                        .map(|id| format!("tool:{id}"))
                        .unwrap_or_else(|| format!("tool:anon:{}", facts.at_ms));
                    entry.insert(key, facts.at_ms);
                }
            }
            "PostToolUse" | "PostToolUseFailure" => {
                if let Some(id) = facts.tool_use_id.as_deref() {
                    entry.remove(&format!("tool:{id}"));
                }
            }
            "SubagentStart" => {
                let key = facts
                    .subagent_id
                    .as_deref()
                    .map(|id| format!("sub:{id}"))
                    .unwrap_or_else(|| format!("sub:anon:{}", facts.at_ms));
                entry.insert(key, facts.at_ms);
            }
            "SubagentStop" => {
                if let Some(id) = facts.subagent_id.as_deref() {
                    entry.remove(&format!("sub:{id}"));
                } else if let Some(first_sub) = entry
                    .keys()
                    .find(|k| k.starts_with("sub:"))
                    .cloned()
                {
                    entry.remove(&first_sub);
                }
            }
            "Stop" => {
                entry.retain(|k, _| !k.starts_with("tool:"));
            }
            _ => {}
        }
        if entry.is_empty() {
            self.by_agent.remove(&facts.agent_id);
        }
    }

    pub fn open(&self, agent_id: &str, now_ms: u64) -> bool {
        self.by_agent.get(agent_id).is_some_and(|map| {
            map.values()
                .any(|&at| now_ms.saturating_sub(at) < BACKGROUND_SUBAGENT_TTL_MS)
        })
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.by_agent.remove(agent_id);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchPhase {
    Idle,
    HandoffOut,
    ReadyToClear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeStep {
    Handoff { work: Option<String> },
    Send { cleared: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearGate {
    Go,
    Hold,
    Drop,
}

#[derive(Debug, Clone)]
pub struct WakeFacts {
    pub rule: SwitchClearRule,
    pub wake_work: Option<String>,
    pub can_clear: bool,
}

#[derive(Debug, Clone)]
struct AgentSwitchState {
    session_started_at: u64,
    session_id: Option<String>,
    on_work: Option<String>,
    phase: SwitchPhase,
    phase_at: u64,
    cleared_lead_pending: bool,
    last_cleared_at: u64,
}

impl AgentSwitchState {
    fn new(now_ms: u64) -> Self {
        Self {
            session_started_at: now_ms,
            session_id: None,
            on_work: None,
            phase: SwitchPhase::Idle,
            phase_at: 0,
            cleared_lead_pending: false,
            last_cleared_at: 0,
        }
    }
}

#[derive(Default)]
pub struct SwitchClear {
    pub background: BackgroundWork,
    by_agent: HashMap<String, AgentSwitchState>,
}

impl SwitchClear {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note_spawn(&mut self, agent_id: &str, now_ms: u64) {
        self.background.forget(agent_id);
        self.by_agent
            .insert(agent_id.to_string(), AgentSwitchState::new(now_ms));
    }

    pub fn note_hook(&mut self, facts: &HookFacts) {
        self.background.note_hook(facts);
        let st = self
            .by_agent
            .entry(facts.agent_id.clone())
            .or_insert_with(|| AgentSwitchState::new(facts.at_ms));
        match facts.event.as_str() {
            "SessionStart" => {
                let changed_sid = facts
                    .session_id
                    .as_deref()
                    .is_some_and(|sid| st.session_id.as_deref() != Some(sid));
                if let Some(sid) = &facts.session_id {
                    st.session_id = Some(sid.clone());
                }
                if changed_sid || st.session_started_at == 0 {
                    st.session_started_at = facts.at_ms;
                }
                if st.phase != SwitchPhase::Idle {
                    st.phase = SwitchPhase::Idle;
                    st.phase_at = facts.at_ms;
                    st.on_work = None;
                    st.cleared_lead_pending = true;
                    st.last_cleared_at = facts.at_ms;
                }
            }
            "Stop" if st.phase == SwitchPhase::HandoffOut => {
                st.phase = SwitchPhase::ReadyToClear;
                st.phase_at = facts.at_ms;
            }
            _ => {}
        }
    }

    pub fn phase(&self, agent_id: &str) -> SwitchPhase {
        self.by_agent
            .get(agent_id)
            .map(|s| s.phase)
            .unwrap_or(SwitchPhase::Idle)
    }

    pub fn on_work(&self, agent_id: &str) -> Option<&str> {
        self.by_agent
            .get(agent_id)
            .and_then(|s| s.on_work.as_deref())
    }

    pub fn wake_step(&mut self, agent_id: &str, facts: &WakeFacts, now_ms: u64) -> WakeStep {
        let Some(st) = self.by_agent.get_mut(agent_id) else {
            return WakeStep::Send { cleared: false };
        };
        if st.cleared_lead_pending {
            return WakeStep::Send { cleared: true };
        }
        if self.due(st, facts, now_ms) && !self.background.open(agent_id, now_ms) {
            return WakeStep::Handoff {
                work: st.on_work.clone(),
            };
        }
        WakeStep::Send { cleared: false }
    }

    fn due(&self, st: &AgentSwitchState, facts: &WakeFacts, now_ms: u64) -> bool {
        if !facts.rule.enabled || !facts.can_clear || st.phase != SwitchPhase::Idle {
            return false;
        }
        let Some(next_work) = facts.wake_work.as_deref() else {
            return false;
        };
        let Some(cur_work) = st.on_work.as_deref() else {
            return false;
        };
        if cur_work == next_work {
            return false;
        }
        if now_ms.saturating_sub(st.session_started_at) < facts.rule.min_wait_ms {
            return false;
        }
        if st.last_cleared_at > 0
            && now_ms.saturating_sub(st.last_cleared_at) < facts.rule.cooldown_ms
        {
            return false;
        }
        true
    }

    pub fn left_to_window(
        &self,
        agent_id: &str,
        facts: &WakeFacts,
        mail_oldest_ms: u64,
        stall_ms: u64,
        now_ms: u64,
    ) -> bool {
        let Some(st) = self.by_agent.get(agent_id) else {
            return false;
        };
        if st.phase == SwitchPhase::ReadyToClear && self.background.open(agent_id, now_ms) {
            return true;
        }
        if st.phase == SwitchPhase::HandoffOut
            && now_ms.saturating_sub(st.phase_at) < HANDOFF_TIMEOUT_MS
        {
            return true;
        }
        self.due(st, facts, now_ms)
            && mail_oldest_ms > 0
            && now_ms.saturating_sub(mail_oldest_ms) < stall_ms
    }

    pub fn abandon(&mut self, agent_id: &str) {
        if let Some(st) = self.by_agent.get_mut(agent_id)
            && st.phase != SwitchPhase::Idle
        {
            st.phase = SwitchPhase::Idle;
            st.phase_at = 0;
        }
    }

    pub fn note_typed(
        &mut self,
        agent_id: Option<&str>,
        kind: TypedKind,
        outcome: &str,
        press_only: bool,
        wake_work: Option<String>,
        now_ms: u64,
    ) {
        if press_only {
            return;
        }
        let accepted = matches!(outcome, "submitted" | "unconfirmed");
        if !accepted {
            return;
        }
        let Some(agent_id) = agent_id else {
            return;
        };
        let st = self
            .by_agent
            .entry(agent_id.to_string())
            .or_insert_with(|| AgentSwitchState::new(now_ms));
        match kind {
            TypedKind::Handoff => {
                st.phase = SwitchPhase::HandoffOut;
                st.phase_at = now_ms;
            }
            TypedKind::Clear => {
                st.phase = SwitchPhase::Idle;
                st.phase_at = now_ms;
                st.session_started_at = now_ms;
                st.last_cleared_at = now_ms;
                st.on_work = None;
                st.cleared_lead_pending = true;
            }
            TypedKind::Wake => {
                st.cleared_lead_pending = false;
                if let Some(work) = wake_work {
                    st.on_work = Some(work);
                }
            }
            TypedKind::Other => {}
        }
    }

    pub fn clear_gate(&mut self, agent_id: &str, now_ms: u64) -> ClearGate {
        if self.background.open(agent_id, now_ms) {
            return ClearGate::Hold;
        }
        let Some(st) = self.by_agent.get_mut(agent_id) else {
            return ClearGate::Go;
        };
        if st.phase == SwitchPhase::HandoffOut
            && now_ms.saturating_sub(st.phase_at) >= HANDOFF_TIMEOUT_MS
        {
            st.phase = SwitchPhase::ReadyToClear;
        }
        ClearGate::Go
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.background.forget(agent_id);
        self.by_agent.remove(agent_id);
    }
}

#[derive(Debug, Clone)]
struct CodexSizeAgent {
    checked_at: u64,
    path: String,
    bytes: u64,
    due: bool,
    handoff_out: bool,
}

#[derive(Default)]
pub struct CodexSizeClear {
    by_agent: HashMap<String, CodexSizeAgent>,
    done_paths: HashSet<String>,
}

impl CodexSizeClear {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check<F>(
        &mut self,
        agent_id: &str,
        transcript_path: Option<&str>,
        is_codex: bool,
        stat_bytes: F,
        now_ms: u64,
    ) -> bool
    where
        F: FnOnce(&str) -> Option<u64>,
    {
        if !is_codex {
            return false;
        }
        let Some(path) = transcript_path.filter(|p| !p.is_empty()) else {
            return false;
        };
        if self.done_paths.contains(path) {
            return false;
        }
        if let Some(prev) = self.by_agent.get(agent_id)
            && prev.path == path
            && now_ms.saturating_sub(prev.checked_at) < CODEX_SIZE_CHECK_MS
        {
            return prev.due;
        }
        let Some(bytes) = stat_bytes(path) else {
            return false;
        };
        let due = bytes >= CODEX_ROLLOUT_CLEAR_BYTES;
        self.by_agent.insert(
            agent_id.to_string(),
            CodexSizeAgent {
                checked_at: now_ms,
                path: path.to_string(),
                bytes,
                due,
                handoff_out: self
                    .by_agent
                    .get(agent_id)
                    .filter(|a| a.path == path)
                    .is_some_and(|a| a.handoff_out),
            },
        );
        due
    }

    pub fn handoff_due(
        &self,
        agent_id: &str,
        phase: SwitchPhase,
        open_background: bool,
        can_clear: bool,
    ) -> bool {
        if !can_clear || open_background || phase != SwitchPhase::Idle {
            return false;
        }
        self.by_agent
            .get(agent_id)
            .is_some_and(|a| a.due && !a.handoff_out && !self.done_paths.contains(&a.path))
    }

    pub fn note_typed(
        &mut self,
        agent_id: Option<&str>,
        kind: TypedKind,
        outcome: &str,
        press_only: bool,
    ) -> Option<u64> {
        if press_only || !matches!(outcome, "submitted" | "unconfirmed") {
            return None;
        }
        let agent_id = agent_id?;
        let entry = self.by_agent.get_mut(agent_id)?;
        match kind {
            TypedKind::Handoff => {
                if entry.due {
                    entry.handoff_out = true;
                }
                None
            }
            TypedKind::Clear => {
                if entry.due {
                    entry.due = false;
                    entry.handoff_out = false;
                    self.done_paths.insert(entry.path.clone());
                    Some(entry.bytes)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.by_agent.remove(agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switch_clear_handoff_then_clear_and_cleared_lead() {
        let mut sc = SwitchClear::new();
        sc.note_spawn("eng-1", 1000);
        sc.note_typed(
            Some("eng-1"),
            TypedKind::Wake,
            "submitted",
            false,
            Some("work-A".into()),
            2000,
        );
        let facts = WakeFacts {
            rule: SwitchClearRule {
                enabled: true,
                min_wait_ms: 600_000,
                cooldown_ms: 600_000,
            },
            wake_work: Some("work-B".into()),
            can_clear: true,
        };
        assert_eq!(
            sc.wake_step("eng-1", &facts, 700_000),
            WakeStep::Handoff {
                work: Some("work-A".into())
            }
        );
        let handoff = handoff_request_text(Some("work-A"), "/tmp/handoff.md");
        assert_eq!(typed_kind(&handoff, Some("/clear")), TypedKind::Handoff);
        sc.note_typed(Some("eng-1"), TypedKind::Handoff, "submitted", false, None, 700_100);
        assert_eq!(sc.phase("eng-1"), SwitchPhase::HandoffOut);
        assert_eq!(sc.clear_gate("eng-1", 700_200), ClearGate::Go);
        sc.note_typed(Some("eng-1"), TypedKind::Clear, "submitted", false, None, 700_300);
        assert_eq!(sc.phase("eng-1"), SwitchPhase::Idle);
        assert_eq!(
            sc.wake_step("eng-1", &facts, 700_400),
            WakeStep::Send { cleared: true }
        );
    }

    #[test]
    fn switch_clear_holds_on_open_subagent() {
        let mut sc = SwitchClear::new();
        sc.note_spawn("eng-1", 1000);
        sc.note_hook(&HookFacts {
            agent_id: "eng-1".into(),
            event: "PreToolUse".into(),
            session_id: None,
            tool: Some("Agent".into()),
            tool_use_id: Some("tu-1".into()),
            subagent_id: None,
            at_ms: 2000,
        });
        assert_eq!(sc.clear_gate("eng-1", 3000), ClearGate::Hold);
        sc.note_hook(&HookFacts {
            agent_id: "eng-1".into(),
            event: "PostToolUse".into(),
            session_id: None,
            tool: Some("Agent".into()),
            tool_use_id: Some("tu-1".into()),
            subagent_id: None,
            at_ms: 4000,
        });
        assert_eq!(sc.clear_gate("eng-1", 5000), ClearGate::Go);
    }

    #[test]
    fn codex_size_clear_triggers_once_per_oversized_rollout() {
        let mut cs = CodexSizeClear::new();
        assert!(!cs.check(
            "codex-1",
            Some("/tmp/rollout-1.jsonl"),
            true,
            |_| Some(CODEX_ROLLOUT_CLEAR_BYTES - 1),
            10_000,
        ));
        assert!(cs.check(
            "codex-1",
            Some("/tmp/rollout-1.jsonl"),
            true,
            |_| Some(CODEX_ROLLOUT_CLEAR_BYTES + 10),
            80_000,
        ));
        assert!(cs.handoff_due("codex-1", SwitchPhase::Idle, false, true));
        cs.note_typed(Some("codex-1"), TypedKind::Handoff, "submitted", false);
        assert!(!cs.handoff_due("codex-1", SwitchPhase::Idle, false, true));
        let rotated = cs.note_typed(Some("codex-1"), TypedKind::Clear, "submitted", false);
        assert_eq!(rotated, Some(CODEX_ROLLOUT_CLEAR_BYTES + 10));
        assert!(!cs.check(
            "codex-1",
            Some("/tmp/rollout-1.jsonl"),
            true,
            |_| Some(CODEX_ROLLOUT_CLEAR_BYTES + 100),
            200_000,
        ));
    }
}
