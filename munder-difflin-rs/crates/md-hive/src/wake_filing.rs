//! WakeFiling: automatically files wake-printed inbox mail to `inbox/.done/`
//! when the agent's turn ends, mirroring `src/main/wakeFiling.ts` and
//! `src/shared/hiveNudge.ts`.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub const WAKE_BODY_MAX: usize = 4_000;
pub const WAKE_TOTAL_MAX: usize = 12_000;
pub const WAKE_QUIET_MS: u64 = 12_000;
pub const WAKE_OPEN_TURN_MS: u64 = 15 * 60_000;
const PROMPT_TAIL_CHARS: usize = 48;

const NUDGE_HEAD: &str = "You have new hive inbox message(s)";
const FILED_LEAD: &str = "These messages are filed for you when your turn ends: ";
const FILED_TAIL: &str = ". Do not open or move the inbox files for them.";
const YOURS_LEAD: &str = "Yours to file: ";
const YOURS_TAIL: &str = ". Move each of those files from your inbox to inbox/.done/ when handled.";
const WAKE_END: &str = "Open a file only where a path is given. Do not send a reply that only acknowledges. Act autonomously; only message god if you genuinely need a decision.";
const PERSON_LINE: &str = "who: the person, who reads your screen. Write your answer to them in full here on your screen, and send it to them by mail as well.";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WakeIds {
    pub filed: Vec<String>,
    pub yours: Vec<String>,
}

pub fn is_safe_wake_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 {
        return false;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphanumeric() || first == b'_' || first == b'-') {
        return false;
    }
    bytes
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn typeable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            '\u{2028}' | '\u{2029}' => out.push('\n'),
            '\t' => out.push_str("  "),
            '\n' => out.push('\n'),
            c if (c as u32) <= 0x1f || (0x7f..=0x9f).contains(&(c as u32)) => {}
            c => out.push(c),
        }
    }
    out
}

fn one_line(text: &str, max: usize) -> String {
    let cleaned = typeable(text);
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(max).collect()
}

fn file_of(inbox_path: Option<&str>, id: &str) -> String {
    match inbox_path {
        Some(p) if !p.is_empty() => {
            let sep = if p.contains('\\') && !p.contains('/') {
                '\\'
            } else {
                '/'
            };
            if p.ends_with(sep) {
                format!("{p}{id}.json")
            } else {
                format!("{p}{sep}{id}.json")
            }
        }
        _ => format!("inbox/{id}.json in your agent folder"),
    }
}

fn work_id_of(m: &Value) -> String {
    m.get("work")
        .or_else(|| m.get("conversation"))
        .or_else(|| m.get("id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

pub fn wake_options_for_provider(provider: &str) -> (bool, bool) {
    let one_line = provider == "antigravity";
    let has_hooks = matches!(
        provider,
        "claude" | "codex" | "grok" | "gemini" | "antigravity" | "opencode" | "pi"
    );
    (one_line, !has_hooks)
}

pub fn build_inbox_wake(
    messages: &[Value],
    inbox_path: Option<&str>,
    use_one_line: bool,
    self_file: bool,
) -> (String, WakeIds) {
    let mut filed = Vec::new();
    let mut yours = Vec::new();
    let mut blocks = Vec::new();
    let join = if use_one_line { " | " } else { "\n" };
    let head = if messages.is_empty() {
        format!("{NUDGE_HEAD}: 0.")
    } else {
        format!("{NUDGE_HEAD}: {}, printed below.", messages.len())
    };
    let mut used = head.len()
        + FILED_LEAD.len()
        + FILED_TAIL.len()
        + YOURS_LEAD.len()
        + YOURS_TAIL.len()
        + WAKE_END.len()
        + inbox_path.map_or(0, str::len)
        + 24;

    let total = messages.len();
    for (i, m) in messages.iter().enumerate() {
        let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let safe = is_safe_wake_id(id);
        let work = one_line(&work_id_of(m), 128);
        let thread = one_line(
            m.get("conversation").and_then(|v| v.as_str()).unwrap_or(""),
            128,
        );
        let from_raw = m.get("from").and_then(|v| v.as_str()).unwrap_or("");
        let from_disp = {
            let s = one_line(from_raw, 128);
            if s.is_empty() {
                "unknown".to_string()
            } else {
                s
            }
        };
        let act = {
            let s = one_line(m.get("act").and_then(|v| v.as_str()).unwrap_or(""), 32);
            if s.is_empty() {
                "inform".to_string()
            } else {
                s
            }
        };
        let req_reply = m
            .get("requires_reply")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let subject = {
            let s = one_line(m.get("subject").and_then(|v| v.as_str()).unwrap_or(""), 300);
            if s.is_empty() {
                "(none)".to_string()
            } else {
                s
            }
        };
        let is_person = matches!(from_raw, "human" | "person") || from_raw.starts_with("slack:");

        let mut lines = vec![
            format!("[{} of {}]", i + 1, total),
            format!(
                "id: {}",
                if safe {
                    id.to_string()
                } else {
                    let s = one_line(id, 128);
                    if s.is_empty() {
                        "(none)".to_string()
                    } else {
                        s
                    }
                }
            ),
            format!("from: {from_disp}"),
            format!(
                "type: {act}{}",
                if req_reply { " (reply expected)" } else { "" }
            ),
            format!("work: {work}"),
        ];
        if !thread.is_empty() && thread != work {
            lines.push(format!("thread: {thread}"));
        }
        lines.push(format!("subject: {subject}"));
        if is_person {
            lines.push(PERSON_LINE.to_string());
        }

        let body_raw = m.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let body = typeable(body_raw).trim().to_string();
        let mut printed_lines = lines.clone();
        if use_one_line {
            printed_lines.push(format!(
                "body: {}",
                if body.is_empty() { "(empty)" } else { &body }
            ));
        } else if body.is_empty() {
            printed_lines.push("body: (empty)".to_string());
        } else {
            printed_lines.push("body:".to_string());
            for l in body.split('\n') {
                printed_lines.push(format!("> {l}").trim_end().to_string());
            }
        }
        let full = printed_lines.join(join);

        let why = if !safe {
            Some("its file name is not known".to_string())
        } else if body.chars().count() > WAKE_BODY_MAX {
            Some("it is over 4,000 characters".to_string())
        } else if use_one_line && body.contains('\n') {
            Some("it has line breaks this terminal cannot take".to_string())
        } else if used + full.len() + id.len() + 6 > WAKE_TOTAL_MAX {
            Some("this wake is full".to_string())
        } else {
            None
        };

        let block = match &why {
            None => full,
            Some(reason) => {
                let mut fallback = lines;
                if safe {
                    fallback.push(format!(
                        "body: not printed, {reason}. Read {}",
                        file_of(inbox_path, id)
                    ));
                } else {
                    fallback.push(format!(
                        "body: not printed, {reason}. Find it in {} and move it to inbox/.done/ when handled.",
                        inbox_path.unwrap_or("your inbox")
                    ));
                }
                fallback.join(join)
            }
        };

        if safe {
            if why.is_some() || self_file {
                yours.push(id.to_string());
            } else {
                filed.push(id.to_string());
            }
        }
        used += block.len() + id.len() + 6;
        blocks.push(block);
    }

    let mut closing_parts = Vec::new();
    if self_file
        && let Some(p) = inbox_path.filter(|s| !s.is_empty())
        && !yours.is_empty()
    {
        closing_parts.push(format!("Your inbox is {p}."));
    }
    if !filed.is_empty() {
        closing_parts.push(format!("{FILED_LEAD}{}{FILED_TAIL}", filed.join(", ")));
    }
    if !yours.is_empty() {
        closing_parts.push(format!("{YOURS_LEAD}{}{YOURS_TAIL}", yours.join(", ")));
    }
    closing_parts.push(WAKE_END.to_string());
    let closing = closing_parts.join(" ");

    let mut parts = Vec::with_capacity(blocks.len() + 2);
    parts.push(head);
    parts.extend(blocks);
    parts.push(closing);
    let sep = if use_one_line { " || " } else { "\n\n" };
    (parts.join(sep), WakeIds { filed, yours })
}

pub fn wake_ids(text: &str) -> WakeIds {
    let mut rest = text.trim_end();
    if !rest.contains(NUDGE_HEAD) || !rest.ends_with(WAKE_END) {
        return WakeIds::default();
    }
    rest = rest[..rest.len() - WAKE_END.len()].trim_end();

    fn take_list(rest: &mut &str, lead: &str, tail: &str) -> Vec<String> {
        if !rest.ends_with(tail) {
            return Vec::new();
        }
        let Some(at) = rest.rfind(lead) else {
            return Vec::new();
        };
        let slice = &rest[at + lead.len()..rest.len() - tail.len()];
        let ids: Vec<String> = slice.split(", ").map(ToString::to_string).collect();
        if ids.is_empty() || !ids.iter().all(|id| is_safe_wake_id(id)) {
            return Vec::new();
        }
        *rest = rest[..at].trim_end();
        ids
    }

    let yours = take_list(&mut rest, YOURS_LEAD, YOURS_TAIL);
    let filed = take_list(&mut rest, FILED_LEAD, FILED_TAIL);
    WakeIds { filed, yours }
}

fn prompt_tail(prompt: &str) -> String {
    let compact: String = prompt.chars().filter(|c| !c.is_whitespace()).collect();
    let total = compact.chars().count();
    compact
        .chars()
        .skip(total.saturating_sub(PROMPT_TAIL_CHARS))
        .collect()
}

fn tail_matches(hook_tail: &str, our_tail: &str) -> bool {
    !hook_tail.is_empty() && !our_tail.is_empty() && hook_tail.ends_with(our_tail)
}

#[derive(Debug, Clone)]
struct TypedWake {
    filed: Vec<String>,
    named: Vec<String>,
    typed_at: u64,
    proved: bool,
    tail: String,
}

#[derive(Default)]
pub struct WakeFiling {
    awaiting: HashMap<String, Vec<TypedWake>>,
    in_box: HashMap<String, TypedWake>,
    named: HashMap<String, HashSet<String>>,
    turn_end_at: HashMap<String, u64>,
    last_hook: HashMap<String, (u64, String)>,
    dropped: HashMap<String, Vec<TypedWake>>,
    doubt: HashMap<String, Vec<TypedWake>>,
}

fn is_turn_hook(event: &str) -> bool {
    matches!(
        event,
        "UserPromptSubmit"
            | "PreToolUse"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "Stop"
            | "StopFailure"
            | "SubagentStart"
            | "SubagentStop"
            | "PreInvocation"
            | "PostInvocation"
    )
}

impl WakeFiling {
    pub fn new() -> Self {
        Self::default()
    }

    fn record_named(&mut self, agent_id: &str, ids: &[String]) {
        let set = self.named.entry(agent_id.to_string()).or_default();
        for id in ids {
            set.insert(id.clone());
        }
    }

    fn submitted(&mut self, agent_id: &str, wake: TypedWake) -> Vec<String> {
        self.in_box.remove(agent_id);
        if wake.proved {
            self.record_named(agent_id, &wake.named);
        }
        let typed_at = wake.typed_at;
        self.awaiting
            .entry(agent_id.to_string())
            .or_default()
            .push(wake);
        if self.turn_end_at.get(agent_id).copied().unwrap_or(0) >= typed_at {
            self.release(agent_id)
        } else {
            Vec::new()
        }
    }

    pub fn note_typed(
        &mut self,
        agent_id: &str,
        text: &str,
        outcome: &str,
        elapsed_ms: u64,
        press_only: bool,
        now_ms: u64,
    ) -> Vec<String> {
        if agent_id.is_empty() {
            return Vec::new();
        }
        let typed_at = now_ms.saturating_sub(elapsed_ms);
        if press_only {
            let Some(mut box_wake) = self.in_box.get(agent_id).cloned() else {
                return Vec::new();
            };
            if outcome == "gone" {
                self.in_box.remove(agent_id);
                return Vec::new();
            }
            if outcome == "submitted" || outcome == "unconfirmed" {
                box_wake.typed_at = typed_at;
                box_wake.proved = outcome == "submitted";
                return self.submitted(agent_id, box_wake);
            }
            return Vec::new();
        }

        if matches!(outcome, "held" | "gone") {
            return Vec::new();
        }
        let ids = wake_ids(text);
        let box_wake = self.in_box.get(agent_id).cloned();
        if ids.filed.is_empty() && ids.yours.is_empty() && box_wake.is_none() {
            return Vec::new();
        }

        let mut filed = ids.filed;
        let mut named: Vec<String> = filed.iter().chain(ids.yours.iter()).cloned().collect();
        if let Some(bw) = box_wake {
            for id in bw.filed {
                if !filed.contains(&id) {
                    filed.push(id);
                }
            }
            for id in bw.named {
                if !named.contains(&id) {
                    named.push(id);
                }
            }
        }
        let wake = TypedWake {
            filed,
            named,
            typed_at,
            proved: outcome == "submitted",
            tail: prompt_tail(text),
        };
        if outcome == "stuck" {
            self.in_box.insert(agent_id.to_string(), wake);
            return Vec::new();
        }
        self.submitted(agent_id, wake)
    }

    fn take_back(&mut self, agent_id: &str, at: u64, seen_prompts: &[(u64, String)]) {
        let prompt_seen = |since: u64, tail: &str| -> bool {
            seen_prompts
                .iter()
                .any(|(p_at, p_tail)| *p_at >= since && tail_matches(p_tail, tail))
        };

        let mut taken = Vec::new();
        if let Some(list) = self.dropped.remove(agent_id) {
            let mut left = Vec::new();
            for w in list {
                if at.saturating_sub(w.typed_at) >= WAKE_OPEN_TURN_MS {
                    continue;
                }
                if prompt_seen(w.typed_at, &w.tail) {
                    taken.push(w);
                } else {
                    left.push(w);
                }
            }
            if !left.is_empty() {
                self.dropped.insert(agent_id.to_string(), left);
            }
        }

        if let Some(doubted) = self.doubt.remove(agent_id) {
            let mut left = Vec::new();
            for w in doubted {
                if prompt_seen(w.typed_at, &w.tail) {
                    taken.push(w);
                } else {
                    left.push(w);
                }
            }
            if !left.is_empty() {
                self.doubt.insert(agent_id.to_string(), left);
            }
        }

        for mut w in taken {
            w.typed_at = at;
            w.proved = true;
            self.record_named(agent_id, &w.named);
            self.awaiting
                .entry(agent_id.to_string())
                .or_default()
                .push(w);
        }
    }

    pub fn note_hook(
        &mut self,
        agent_id: &str,
        event: &str,
        source: Option<&str>,
        seen_prompts: &[(u64, String)],
        now_ms: u64,
    ) -> Vec<String> {
        if agent_id.is_empty() || event.is_empty() {
            return Vec::new();
        }
        let prompt_seen = |since: u64, tail: &str| -> bool {
            seen_prompts
                .iter()
                .any(|(p_at, p_tail)| *p_at >= since && tail_matches(p_tail, tail))
        };

        if event == "UserPromptSubmit" {
            if let Some(mut box_wake) = self.in_box.get(agent_id).cloned()
                && prompt_seen(box_wake.typed_at, &box_wake.tail)
            {
                box_wake.typed_at = now_ms;
                box_wake.proved = true;
                let _ = self.submitted(agent_id, box_wake);
            }
            self.take_back(agent_id, now_ms, seen_prompts);

            if let Some(waits) = self.awaiting.remove(agent_id) {
                let mut kept = Vec::new();
                let mut dropped_now = Vec::new();
                for mut w in waits {
                    if w.proved || prompt_seen(w.typed_at, &w.tail) {
                        if !w.proved {
                            w.proved = true;
                            self.record_named(agent_id, &w.named);
                        }
                        kept.push(w);
                    } else {
                        dropped_now.push(w);
                    }
                }
                if !dropped_now.is_empty() {
                    self.dropped
                        .entry(agent_id.to_string())
                        .or_default()
                        .extend(dropped_now);
                }
                if !kept.is_empty() {
                    self.awaiting.insert(agent_id.to_string(), kept);
                }
            }
        }

        if event == "SessionStart" && source != Some("compact") {
            if let Some(box_wake) = self.in_box.remove(agent_id) {
                self.dropped
                    .entry(agent_id.to_string())
                    .or_default()
                    .push(box_wake);
            }
            if let Some(waits) = self.awaiting.remove(agent_id) {
                let mut kept = Vec::new();
                let mut dropped_now = Vec::new();
                for w in waits {
                    if w.proved {
                        kept.push(w);
                    } else {
                        dropped_now.push(w);
                    }
                }
                if !dropped_now.is_empty() {
                    self.dropped
                        .entry(agent_id.to_string())
                        .or_default()
                        .extend(dropped_now);
                }
                if !kept.is_empty() {
                    self.awaiting.insert(agent_id.to_string(), kept);
                }
            }
        }

        if !is_turn_hook(event) {
            return Vec::new();
        }
        self.last_hook
            .insert(agent_id.to_string(), (now_ms, event.to_string()));

        if event == "PreInvocation"
            && let Some(waits) = self.awaiting.get_mut(agent_id)
        {
            let mut to_name = Vec::new();
            for w in waits.iter_mut() {
                if !w.proved {
                    w.proved = true;
                    to_name.extend(w.named.clone());
                }
            }
            if !to_name.is_empty() {
                self.record_named(agent_id, &to_name);
            }
        }

        if matches!(event, "Stop" | "PostInvocation") {
            self.turn_end_at.insert(agent_id.to_string(), now_ms);
            return self.release(agent_id);
        }
        Vec::new()
    }

    pub fn note_quiet(&mut self, agent_id: &str, last_output_at: u64, now_ms: u64) -> Vec<String> {
        let Some(waits) = self.awaiting.get(agent_id) else {
            return Vec::new();
        };
        if waits.is_empty() || !waits.iter().all(|w| w.proved) {
            return Vec::new();
        }
        if last_output_at == 0 || now_ms.saturating_sub(last_output_at) < WAKE_QUIET_MS {
            return Vec::new();
        }
        let first = waits.iter().map(|w| w.typed_at).min().unwrap_or(0);
        let latest = waits.iter().map(|w| w.typed_at).max().unwrap_or(0);
        if now_ms.saturating_sub(latest) < WAKE_QUIET_MS {
            return Vec::new();
        }
        if let Some((hook_at, hook_event)) = self.last_hook.get(agent_id)
            && *hook_at >= first
        {
            if hook_event == "StopFailure" {
                return Vec::new();
            }
            if now_ms.saturating_sub(*hook_at) < WAKE_OPEN_TURN_MS {
                return Vec::new();
            }
        }
        self.release(agent_id)
    }

    fn release(&mut self, agent_id: &str) -> Vec<String> {
        let Some(waits) = self.awaiting.remove(agent_id) else {
            return Vec::new();
        };
        if waits.is_empty() {
            return Vec::new();
        }
        let mut unproved = Vec::new();
        let mut filed_ids = Vec::new();
        let mut seen_filed = HashSet::new();
        for w in waits {
            if w.proved {
                for id in w.filed {
                    if seen_filed.insert(id.clone()) {
                        filed_ids.push(id);
                    }
                }
            } else {
                self.record_named(agent_id, &w.named);
                unproved.push(w);
            }
        }
        if !unproved.is_empty() {
            self.doubt
                .entry(agent_id.to_string())
                .or_default()
                .extend(unproved);
        }
        filed_ids
    }

    pub fn waiting(&self) -> Vec<String> {
        self.awaiting.keys().cloned().collect()
    }

    pub fn pending(&mut self, agent_id: &str, inbox: &[Value]) -> Vec<Value> {
        let Some(named) = self.named.get_mut(agent_id) else {
            return inbox.to_vec();
        };
        if named.is_empty() {
            return inbox.to_vec();
        }
        let live: HashSet<&str> = inbox
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
            .collect();
        named.retain(|id| live.contains(id.as_str()));
        inbox
            .iter()
            .filter(|m| {
                let id = m.get("id").and_then(|v| v.as_str());
                !id.is_some_and(|s| named.contains(s))
            })
            .cloned()
            .collect()
    }

    pub fn unconfirmed(&mut self, agent_id: &str, inbox: &[Value]) -> Vec<String> {
        let Some(list) = self.doubt.get_mut(agent_id) else {
            return Vec::new();
        };
        if list.is_empty() {
            return Vec::new();
        }
        let live: HashSet<&str> = inbox
            .iter()
            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
            .collect();
        list.retain(|w| w.filed.iter().any(|id| live.contains(id.as_str())));
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        for w in list.iter() {
            for id in &w.filed {
                if live.contains(id.as_str()) && seen.insert(id.clone()) {
                    ids.push(id.clone());
                }
            }
        }
        if list.is_empty() {
            self.doubt.remove(agent_id);
        }
        ids
    }

    pub fn retry(&mut self, agent_id: &str) -> Vec<String> {
        let Some(again) = self.doubt.remove(agent_id) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        if let Some(named) = self.named.get_mut(agent_id) {
            for w in &again {
                for id in &w.named {
                    named.remove(id);
                }
            }
        }
        for w in again {
            for id in w.filed {
                if seen.insert(id.clone()) {
                    out.push(id);
                }
            }
        }
        out
    }

    pub fn forget(&mut self, agent_id: &str) {
        self.awaiting.remove(agent_id);
        self.doubt.remove(agent_id);
        self.in_box.remove(agent_id);
        self.dropped.remove(agent_id);
        self.named.remove(agent_id);
        self.turn_end_at.remove(agent_id);
        self.last_hook.remove(agent_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_and_parse_wake_ids_roundtrip() {
        let msgs = vec![json!({
            "id": "out-1795680575998-bootstrap-ready",
            "from": "research-analyst-mwx9tivt",
            "act": "inform",
            "subject": "Bootstrap complete; ready for research tasks",
            "body": "research-analyst-mwx9tivt booted.\nReady."
        })];
        let (text, ids) = build_inbox_wake(&msgs, Some("/tmp/hive/agents/god/inbox"), false, false);
        assert_eq!(ids.filed, vec!["out-1795680575998-bootstrap-ready"]);
        assert!(ids.yours.is_empty());
        assert_eq!(wake_ids(&text), ids);
    }

    #[test]
    fn wake_filing_files_proved_wake_on_stop_and_skips_pending() {
        let mut wf = WakeFiling::new();
        let msgs = vec![json!({
            "id": "out-1-ready",
            "from": "worker-1",
            "act": "inform",
            "subject": "Ready",
            "body": "Standing by"
        })];
        let (text, _) = build_inbox_wake(&msgs, Some("/tmp/inbox"), false, false);

        // Typing wake marks it pending and awaiting Stop
        assert!(wf.note_typed("god", &text, "submitted", 10, false, 1000).is_empty());
        assert!(wf.pending("god", &msgs).is_empty());

        // Stop fires -> files out-1-ready
        let filed = wf.note_hook("god", "Stop", None, &[], 2000);
        assert_eq!(filed, vec!["out-1-ready"]);
    }
}
