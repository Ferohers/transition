use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex as AsyncMutex, oneshot};
use uuid::Uuid;

use crate::manager::{PtyError, PtyManager};

pub type Result<T> = std::result::Result<T, PtyError>;

pub const PTY_SUBMIT_DELAY_MS: u64 = 240;
pub const PTY_RETRY_AFTER_MS: u64 = 4_000;
pub const PTY_SUBMIT_MAX_PRESSES: u32 = 4;
pub const PTY_FINAL_WAIT_MS: u64 = 4_000;
pub const PTY_PROOF_POLL_MS: u64 = 150;
pub const COMPACT_WAIT_MAX_MS: u64 = 240_000;
pub const CLEAR_SEND_SETTLE_MS: u64 = 600;
pub const PROMPT_TAIL_CHARS: usize = 48;
pub const ASK_ROWS: usize = 12;

pub const PROMPT_TAKEN_EVENTS: &[&str] = &["UserPromptSubmit", "PreInvocation"];

#[derive(Debug, Clone, Copy)]
pub struct SubmitTimings {
    pub submit_delay_ms: u64,
    pub retry_after_ms: u64,
    pub max_presses: u32,
    pub final_wait_ms: u64,
    pub proof_poll_ms: u64,
    pub boot_wait_ms: u64,
    pub boot_min_ms: u64,
    pub ready_poll_ms: u64,
    pub clear_settle_ms: u64,
}

impl Default for SubmitTimings {
    fn default() -> Self {
        Self {
            submit_delay_ms: PTY_SUBMIT_DELAY_MS,
            retry_after_ms: PTY_RETRY_AFTER_MS,
            max_presses: PTY_SUBMIT_MAX_PRESSES,
            final_wait_ms: PTY_FINAL_WAIT_MS,
            proof_poll_ms: PTY_PROOF_POLL_MS,
            boot_wait_ms: 6_000,
            boot_min_ms: 1_800,
            ready_poll_ms: 150,
            clear_settle_ms: CLEAR_SEND_SETTLE_MS,
        }
    }
}

impl SubmitTimings {
    pub fn fast_for_test() -> Self {
        Self {
            submit_delay_ms: 15,
            retry_after_ms: 40,
            max_presses: PTY_SUBMIT_MAX_PRESSES,
            final_wait_ms: 40,
            proof_poll_ms: 10,
            boot_wait_ms: 120,
            boot_min_ms: 20,
            ready_poll_ms: 10,
            clear_settle_ms: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PtySubmitRequest {
    pub pty_id: String,
    pub agent_id: Option<String>,
    pub provider: String,
    pub text: String,
    #[serde(default)]
    pub source: String,
    pub settle_ms: Option<u64>,
    #[serde(default)]
    pub press_only: bool,
    #[serde(default)]
    pub idle: Option<bool>,
    #[serde(rename = "ref")]
    pub ref_id: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitResult {
    pub ok: bool,
    pub outcome: String,
    pub proof: String,
    pub presses: u32,
    pub ms: u64,
    pub reason: Option<String>,
    pub chars: usize,
    #[serde(default, skip_serializing_if = "is_false")]
    pub behind: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub moved: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub left: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProbeRequest {
    pub id: String,
    pub pty_id: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProbeReply {
    pub id: String,
    pub lines: Option<Vec<String>>,
    #[serde(default)]
    pub safe: bool,
    #[serde(default = "default_true")]
    pub pooled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Default)]
pub struct TerminalProbes {
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<TerminalProbeReply>>>>,
}

impl TerminalProbes {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        pty_id: &str,
        cols: u16,
        rows: u16,
    ) -> (TerminalProbeRequest, oneshot::Receiver<TerminalProbeReply>) {
        let id = Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        let req = TerminalProbeRequest {
            id,
            pty_id: pty_id.to_string(),
            cols,
            rows,
        };
        (req, rx)
    }

    pub fn resolve(&self, reply: TerminalProbeReply) -> bool {
        if let Some(tx) = self.pending.lock().unwrap().remove(&reply.id) {
            tx.send(reply).is_ok()
        } else {
            false
        }
    }

    pub async fn wait_with_timeout(
        &self,
        id: &str,
        rx: oneshot::Receiver<TerminalProbeReply>,
    ) -> Option<TerminalProbeReply> {
        match tokio::time::timeout(Duration::from_millis(300), rx).await {
            Ok(Ok(reply)) => Some(reply),
            _ => {
                self.pending.lock().unwrap().remove(id);
                None
            }
        }
    }
}

pub trait SubmitHooks: Send + Sync {
    fn prompt_seen(&self, agent_id: &str, since_ms: u64, tail: &str) -> bool;
    fn activity_seen(&self, agent_id: &str, since_ms: u64) -> bool;
    fn event_seen(&self, agent_id: &str, since_ms: u64, events: &[&str]) -> bool;
    fn ask_open(&self, agent_id: &str) -> bool;
    fn compact_running_since(&self, agent_id: &str, since_ms: u64, max_ms: u64, now_ms: u64)
    -> u64;
}

pub struct NoopSubmitHooks;

impl SubmitHooks for NoopSubmitHooks {
    fn prompt_seen(&self, _agent_id: &str, _since_ms: u64, _tail: &str) -> bool {
        false
    }
    fn activity_seen(&self, _agent_id: &str, _since_ms: u64) -> bool {
        false
    }
    fn event_seen(&self, _agent_id: &str, _since_ms: u64, _events: &[&str]) -> bool {
        false
    }
    fn ask_open(&self, _agent_id: &str) -> bool {
        false
    }
    fn compact_running_since(
        &self,
        _agent_id: &str,
        _since_ms: u64,
        _max_ms: u64,
        _now_ms: u64,
    ) -> u64 {
        0
    }
}

pub trait ScreenProbe: Send + Sync {
    fn probe<'a>(
        &'a self,
        pty_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<TerminalProbeReply>> + Send + 'a>>;
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

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

pub fn prompt_tail(text: &str) -> String {
    let cleaned = strip_pasted_content_tags(text);
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

pub fn is_slash_command(text: &str) -> bool {
    let t = text.trim();
    t.starts_with('/') && !t.contains('\n') && !t.contains('\r')
}

pub fn command_proof_events(text: &str) -> &'static [&'static str] {
    let cmd = text
        .trim()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match cmd.as_str() {
        "/compact" | "/compress" => &["PreCompact", "PostCompact"],
        "/clear" | "/new" | "/reset" => &["SessionStart"],
        _ => &[],
    }
}

pub fn is_box_rule(line: &str) -> bool {
    let t = line.trim();
    if t.chars().count() < 8 {
        return false;
    }
    let mut horiz = 0usize;
    let mut corners = 0usize;
    let mut other = 0usize;
    for ch in t.chars() {
        match ch {
            '─' | '━' | '═' | '-' | '_' => horiz += 1,
            '╭' | '╮' | '╰' | '╯' | '┌' | '┐' | '└' | '┘' | '+' | '├' | '┤' | '┠' | '┨'
            | '┝' | '┥' => corners += 1,
            ' ' => {}
            _ => other += 1,
        }
    }
    horiz >= 8 && (other == 0 || (corners > 0 && other <= 16))
}

pub fn input_box_text(lines: &[String]) -> Option<String> {
    let limit = lines.len();
    let start_scan = limit.saturating_sub(24);
    let rules: Vec<usize> = (start_scan..limit).filter(|&i| is_box_rule(&lines[i])).collect();
    if rules.len() >= 2 {
        let top = rules[rules.len() - 2];
        let bot = rules[rules.len() - 1];
        if bot > top + 1 && bot - top <= 14 {
            let inner: Vec<String> = lines[top + 1..bot]
                .iter()
                .map(|l| strip_prompt_chrome(l))
                .filter(|s| !s.is_empty())
                .collect();
            return Some(inner.join(" "));
        }
    }
    // Single bottom prompt marker fallback (`❯ ...` / `› ...`) inside last 6 lines.
    let prompt_scan = limit.saturating_sub(6);
    for l in &lines[prompt_scan..] {
        let trimmed = l.trim();
        if let Some(rest) = trimmed
            .strip_prefix("❯")
            .or_else(|| trimmed.strip_prefix("›"))
        {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn strip_prompt_chrome(line: &str) -> String {
    let mut s = line.trim();
    for border in ['│', '┃', '║'] {
        s = s.trim_start_matches(border).trim_end_matches(border).trim();
    }
    for marker in ["❯", "›", ">"] {
        if let Some(rest) = s.strip_prefix(marker) {
            s = rest.trim();
            break;
        }
    }
    s.to_string()
}

fn is_placeholder_draft(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.is_empty()
        || lower.starts_with("try \"")
        || lower.starts_with("ask anything")
        || lower.starts_with("type your message")
        || lower.starts_with("send a message")
        || lower.starts_with("write a message")
        || lower == "? for shortcuts"
}

pub fn box_has_unsent_draft(lines: &[String]) -> bool {
    input_box_text(lines).is_some_and(|t| !is_placeholder_draft(&t))
}

pub fn submit_still_pending(lines: &[String], expected_text: &str) -> Option<bool> {
    let box_txt = input_box_text(lines)?;
    let box_compact: String = strip_pasted_content_tags(&box_txt)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if box_compact.is_empty() || is_placeholder_draft(&box_txt) {
        return Some(false);
    }
    let tail = prompt_tail(expected_text);
    if tail.is_empty() {
        return Some(false);
    }
    let needle_len = tail.chars().count().min(20);
    let needle: String = tail
        .chars()
        .skip(tail.chars().count().saturating_sub(needle_len))
        .collect();
    Some(tail_matches(&box_compact, &tail) || box_compact.contains(&needle))
}

pub fn prompt_on_screen(lines: &[String]) -> bool {
    let last_rule = lines.iter().rposition(|l| is_box_rule(l));
    let slice = match last_rule {
        Some(idx) if idx + 1 < lines.len() => &lines[idx + 1..],
        _ => lines,
    };
    let tail: Vec<&str> = slice
        .iter()
        .map(String::as_str)
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(ASK_ROWS)
        .collect();
    tail.iter().any(|l| {
        let lower = l.to_ascii_lowercase();
        (lower.contains("do you want to") && l.contains('?'))
            || lower.contains("enter to select")
            || lower.contains("enter to confirm")
            || lower.contains("enter to continue")
            || lower.contains("enter to choose")
            || lower.contains("esc to cancel")
            || lower.contains("escape to cancel")
            || lower.contains("would you like to run")
            || lower.contains("would you like to make")
            || lower.contains("would you like to apply")
            || lower.contains("would you like to allow")
            || lower.contains("allow command?")
            || lower.contains("press enter to confirm or esc to cancel")
            || lower.contains("yes, proceed")
    })
}

#[derive(Debug, Clone)]
struct UnprovedSend {
    tail: String,
    at_ms: u64,
}

#[derive(Clone)]
pub struct PtySubmitter {
    pty: PtyManager,
    timings: SubmitTimings,
    active_refs: Arc<Mutex<HashSet<String>>>,
    withdrawn_refs: Arc<Mutex<HashSet<String>>>,
    pty_locks: Arc<Mutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    unproved: Arc<Mutex<HashMap<String, UnprovedSend>>>,
}

impl PtySubmitter {
    pub fn new(pty: PtyManager) -> Self {
        Self::with_timings(pty, SubmitTimings::default())
    }

    pub fn with_timings(pty: PtyManager, timings: SubmitTimings) -> Self {
        Self {
            pty,
            timings,
            active_refs: Arc::new(Mutex::new(HashSet::new())),
            withdrawn_refs: Arc::new(Mutex::new(HashSet::new())),
            pty_locks: Arc::new(Mutex::new(HashMap::new())),
            unproved: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn pty_lock(&self, pty_id: &str) -> Arc<AsyncMutex<()>> {
        let mut map = self.pty_locks.lock().unwrap();
        map.entry(pty_id.to_string())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone()
    }

    fn is_withdrawn(&self, ref_id: Option<&str>) -> bool {
        ref_id.is_some_and(|r| self.withdrawn_refs.lock().unwrap().contains(r))
    }

    pub fn withdraw(&self, ref_id: &str) -> bool {
        if ref_id.is_empty() {
            return false;
        }
        let active = self.active_refs.lock().unwrap().contains(ref_id);
        self.withdrawn_refs.lock().unwrap().insert(ref_id.to_string());
        active
    }

    pub async fn submit<F>(&self, req: PtySubmitRequest, on_began: F) -> Result<SubmitResult>
    where
        F: Fn(&PtySubmitRequest) + Send + Sync,
    {
        self.submit_with_context(req, None, &NoopSubmitHooks, on_began)
            .await
    }

    pub async fn submit_with_context<F>(
        &self,
        req: PtySubmitRequest,
        probe: Option<&dyn ScreenProbe>,
        hooks: &dyn SubmitHooks,
        on_began: F,
    ) -> Result<SubmitResult>
    where
        F: Fn(&PtySubmitRequest) + Send + Sync,
    {
        let chars = req.text.chars().count();
        let ref_id = req.ref_id.clone();
        if let Some(ref r) = ref_id {
            if self.withdrawn_refs.lock().unwrap().contains(r) {
                return Ok(SubmitResult {
                    ok: false,
                    outcome: "held".into(),
                    proof: "none".into(),
                    presses: 0,
                    ms: 0,
                    reason: Some("withdrawn".into()),
                    chars,
                    behind: false,
                    moved: false,
                    left: false,
                });
            }
            self.active_refs.lock().unwrap().insert(r.clone());
        }

        let pty_lock = self.pty_lock(&req.pty_id);
        let _guard = pty_lock.lock().await;

        let res = self
            .run_submit_locked(&req, probe, hooks, &on_began)
            .await;

        if let Some(ref r) = ref_id {
            self.active_refs.lock().unwrap().remove(r);
            self.withdrawn_refs.lock().unwrap().remove(r);
        }
        res
    }

    async fn run_submit_locked<F>(
        &self,
        req: &PtySubmitRequest,
        probe: Option<&dyn ScreenProbe>,
        hooks: &dyn SubmitHooks,
        on_began: &F,
    ) -> Result<SubmitResult>
    where
        F: Fn(&PtySubmitRequest) + Send + Sync,
    {
        let start = Instant::now();
        let chars = req.text.chars().count();
        let eff_agent = req
            .agent_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| req.pty_id.strip_prefix("pty-"));
        let make_res = |outcome: &str,
                        proof: &str,
                        presses: u32,
                        reason: Option<&str>,
                        behind: bool,
                        moved: bool,
                        left: bool|
         -> SubmitResult {
            let ok = matches!(outcome, "submitted" | "unconfirmed");
            SubmitResult {
                ok,
                outcome: outcome.to_string(),
                proof: proof.to_string(),
                presses,
                ms: start.elapsed().as_millis() as u64,
                reason: reason.map(ToString::to_string),
                chars,
                behind,
                moved,
                left,
            }
        };

        let Some(init_info) = self.pty.info(&req.pty_id) else {
            return Ok(make_res(
                "gone",
                "none",
                0,
                Some("no-pty"),
                false,
                false,
                false,
            ));
        };
        let pty_started_at = init_info.started_at;

        // Wait for PTY boot settle, active compaction, or open approval prompt.
        let ready_deadline = Instant::now() + Duration::from_millis(self.timings.boot_wait_ms);
        loop {
            let Some(info) = self.pty.info(&req.pty_id) else {
                return Ok(make_res(
                    "gone",
                    "none",
                    0,
                    Some("pty-exited"),
                    false,
                    false,
                    false,
                ));
            };
            if self.is_withdrawn(req.ref_id.as_deref()) {
                return Ok(make_res(
                    "held",
                    "none",
                    0,
                    Some("withdrawn"),
                    false,
                    false,
                    false,
                ));
            }
            let now = unix_now_ms();
            let elapsed_boot = now.saturating_sub(info.started_at);
            let booted = info.has_output && elapsed_boot >= self.timings.boot_min_ms;
            let compacting = !req.press_only
                && eff_agent.is_some_and(|a| {
                    hooks.compact_running_since(a, pty_started_at, COMPACT_WAIT_MAX_MS, now) > 0
                });
            let asking = !req.press_only && eff_agent.is_some_and(|a| hooks.ask_open(a));
            if booted && !compacting && !asking {
                break;
            }
            if Instant::now() >= ready_deadline {
                if asking {
                    return Ok(make_res(
                        "held",
                        "none",
                        0,
                        Some("ask-open"),
                        true,
                        false,
                        false,
                    ));
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(self.timings.ready_poll_ms)).await;
        }

        // Check earlier unproved send on this PTY before typing new content.
        if !req.press_only {
            let prev_unproved = self.unproved.lock().unwrap().get(&req.pty_id).cloned();
            if let Some(prev) = prev_unproved {
                let proved_since = eff_agent.is_some_and(|a| {
                    hooks.prompt_seen(a, prev.at_ms, &prev.tail)
                        || hooks.activity_seen(a, prev.at_ms)
                });
                if proved_since {
                    self.unproved.lock().unwrap().remove(&req.pty_id);
                } else if let Some(p) = probe
                    && let Some(reply) = p.probe(&req.pty_id).await
                    && let Some(ref lines) = reply.lines
                    && submit_still_pending(lines, &prev.tail) == Some(true)
                {
                    let _ = self.pty.write(&req.pty_id, "\r");
                    tokio::time::sleep(Duration::from_millis(self.timings.clear_settle_ms)).await;
                    if let Some(reply2) = p.probe(&req.pty_id).await
                        && let Some(ref lines2) = reply2.lines
                        && submit_still_pending(lines2, &prev.tail) == Some(true)
                    {
                        return Ok(make_res(
                            "held",
                            "none",
                            0,
                            Some("earlier-unproved"),
                            false,
                            false,
                            false,
                        ));
                    }
                    self.unproved.lock().unwrap().remove(&req.pty_id);
                } else {
                    self.unproved.lock().unwrap().remove(&req.pty_id);
                }
            }
        }

        // Pre-type screen safety probe (menu / approval prompt / live draft in box).
        if !req.press_only
            && let Some(p) = probe
            && let Some(reply) = p.probe(&req.pty_id).await
            && let Some(ref lines) = reply.lines
        {
            if req.source != "human" && prompt_on_screen(lines) {
                return Ok(make_res(
                    "held",
                    "none",
                    0,
                    Some("prompt-on-screen"),
                    true,
                    false,
                    false,
                ));
            }
            if req.source != "queue" && !reply.safe && box_has_unsent_draft(lines) {
                return Ok(make_res(
                    "held",
                    "none",
                    0,
                    Some("draft-in-box"),
                    false,
                    false,
                    false,
                ));
            }
        }

        let began_ms = unix_now_ms();
        on_began(req);

        if !req.press_only && !req.text.is_empty() {
            let bracketed = req.text.contains('\n')
                || req.text.contains('\r')
                || req.text.len() > 200
                || req.provider == "claude";
            let payload = if bracketed {
                format!("\x1b[200~{}\x1b[201~", req.text)
            } else {
                req.text.clone()
            };
            self.pty.write(&req.pty_id, &payload)?;
            tokio::time::sleep(Duration::from_millis(self.timings.submit_delay_ms)).await;
        }

        if self.is_withdrawn(req.ref_id.as_deref()) {
            return Ok(make_res(
                "held",
                "none",
                0,
                Some("withdrawn"),
                false,
                false,
                false,
            ));
        }

        let tail = prompt_tail(&req.text);
        let slash = !req.press_only && is_slash_command(&req.text);
        let cmd_events = if slash {
            command_proof_events(&req.text)
        } else {
            &[]
        };
        let idle = req.idle.unwrap_or(false);
        let is_wake = req.source == "nudge" || req.source == "queue";

        let mut presses: u32 = 0;
        let mut seen = req.press_only;
        let mut last_pending: Option<bool> = None;
        let mut gone_reads: u32 = 0;
        let mut gone_unpressed = false;
        let mut elsewhere = false;

        if self.press_safe(req, probe).await? {
            presses += 1;
        }

        let hook_channel = eff_agent.is_some_and(|a| hooks.activity_seen(a, pty_started_at));
        let max_attempts = if slash { 1 } else { self.timings.max_presses };
        let mut attempts = 1u32;
        let mut gap = self.timings.retry_after_ms;
        let mut next_at = Instant::now() + Duration::from_millis(gap);
        let mut final_at: Option<Instant> = if slash {
            Some(Instant::now() + Duration::from_millis(self.timings.final_wait_ms))
        } else {
            None
        };

        loop {
            if self.pty.info(&req.pty_id).is_none() {
                return Ok(make_res(
                    "gone",
                    "none",
                    presses,
                    Some("pty-exited"),
                    false,
                    false,
                    false,
                ));
            }
            if self.is_withdrawn(req.ref_id.as_deref()) {
                return Ok(make_res(
                    "held",
                    "none",
                    presses,
                    Some("withdrawn"),
                    false,
                    false,
                    false,
                ));
            }

            if let Some(aid) = eff_agent {
                if !tail.is_empty() && hooks.prompt_seen(aid, began_ms, &tail) {
                    return Ok(make_res("submitted", "hook", presses, None, false, false, false));
                }
                if !cmd_events.is_empty() && hooks.event_seen(aid, began_ms, cmd_events) {
                    return Ok(make_res("submitted", "hook", presses, None, false, false, false));
                }
                if idle && presses > 0 && hooks.event_seen(aid, began_ms, PROMPT_TAKEN_EVENTS) {
                    return Ok(make_res(
                        "submitted",
                        "activity",
                        presses,
                        None,
                        false,
                        false,
                        false,
                    ));
                }
            }

            if let Some(p) = probe
                && let Some(reply) = p.probe(&req.pty_id).await
                && let Some(ref lines) = reply.lines
            {
                let pending = submit_still_pending(lines, &req.text);
                last_pending = pending;
                if pending == Some(true) {
                    seen = true;
                    gone_reads = 0;
                } else if pending == Some(false) && seen {
                    gone_reads += 1;
                    if gone_reads >= 2 {
                        if presses == 0 {
                            gone_unpressed = true;
                        } else {
                            return Ok(make_res(
                                "submitted",
                                "screen",
                                presses,
                                None,
                                false,
                                false,
                                false,
                            ));
                        }
                    }
                } else {
                    gone_reads = 0;
                }
            }

            if gone_unpressed {
                return Ok(make_res(
                    if is_wake { "gone" } else { "unconfirmed" },
                    "none",
                    presses,
                    Some("left-box-unpressed"),
                    false,
                    false,
                    true,
                ));
            }

            let now_inst = Instant::now();
            if let Some(deadline) = final_at
                && now_inst >= deadline
            {
                break;
            }

            if !elsewhere
                && idle
                && presses > 0
                && let Some(aid) = eff_agent
                && hooks.activity_seen(aid, began_ms)
                && !hooks.event_seen(aid, began_ms, PROMPT_TAKEN_EVENTS)
            {
                elsewhere = true;
                if final_at.is_none() {
                    final_at = Some(now_inst + Duration::from_millis(self.timings.final_wait_ms));
                }
            }

            if !elsewhere && now_inst >= next_at && attempts < max_attempts {
                if last_pending != Some(true) && !hook_channel {
                    break;
                }
                if last_pending != Some(false) && self.press_safe(req, probe).await? {
                    presses += 1;
                }
                attempts += 1;
                gap = gap.saturating_mul(2);
                next_at = Instant::now() + Duration::from_millis(gap);
                if attempts >= max_attempts {
                    final_at = Some(Instant::now() + Duration::from_millis(self.timings.final_wait_ms));
                }
            }

            tokio::time::sleep(Duration::from_millis(self.timings.proof_poll_ms)).await;
        }

        if presses == 0 {
            return Ok(make_res(
                "stuck",
                "none",
                presses,
                Some("presses-refused"),
                false,
                false,
                false,
            ));
        }
        if elsewhere {
            return Ok(make_res(
                "unconfirmed",
                "none",
                presses,
                None,
                false,
                true,
                false,
            ));
        }
        if slash {
            return Ok(make_res("unconfirmed", "none", presses, None, false, false, false));
        }

        if last_pending != Some(true)
            && let Some(p) = probe
            && let Some(reply) = p.probe(&req.pty_id).await
            && let Some(ref lines) = reply.lines
            && submit_still_pending(lines, &req.text) == Some(true)
        {
            last_pending = Some(true);
            if self.press_safe(req, probe).await? {
                presses += 1;
            }
            let until = Instant::now() + Duration::from_millis(self.timings.final_wait_ms);
            while Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(self.timings.proof_poll_ms)).await;
                if let Some(aid) = eff_agent {
                    if !tail.is_empty() && hooks.prompt_seen(aid, began_ms, &tail) {
                        return Ok(make_res(
                            "submitted",
                            "hook",
                            presses,
                            None,
                            false,
                            false,
                            false,
                        ));
                    }
                }
                if let Some(reply2) = p.probe(&req.pty_id).await
                    && let Some(ref lines2) = reply2.lines
                    && submit_still_pending(lines2, &req.text) == Some(false)
                {
                    return Ok(make_res(
                        "submitted",
                        "screen",
                        presses,
                        None,
                        false,
                        false,
                        false,
                    ));
                }
            }
        }

        if last_pending == Some(true) || (last_pending.is_none() && hook_channel) {
            return Ok(make_res(
                "stuck",
                "none",
                presses,
                Some(if last_pending == Some(true) {
                    "still-in-box"
                } else {
                    "no-hook-proof"
                }),
                false,
                false,
                false,
            ));
        }

        if !tail.is_empty() && !req.press_only {
            self.unproved.lock().unwrap().insert(
                req.pty_id.clone(),
                UnprovedSend {
                    tail,
                    at_ms: began_ms,
                },
            );
        }

        if let Some(extra_settle) = req.settle_ms.filter(|&ms| ms > 0 && ms <= 10_000) {
            // Preserve short caller settle hint if not already covered.
            let _ = extra_settle;
        }

        Ok(make_res("unconfirmed", "none", presses, None, false, false, false))
    }

    async fn press_safe(
        &self,
        req: &PtySubmitRequest,
        probe: Option<&dyn ScreenProbe>,
    ) -> Result<bool> {
        if self.is_withdrawn(req.ref_id.as_deref()) {
            return Ok(false);
        }
        if let Some(p) = probe
            && let Some(reply) = p.probe(&req.pty_id).await
            && let Some(ref lines) = reply.lines
            && req.source != "human"
            && prompt_on_screen(lines)
        {
            return Ok(false);
        }
        self.pty.write(&req.pty_id, "\r")?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::SpawnOptions;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MockHooks {
        prompts: Mutex<Vec<(u64, String)>>,
        activity_at: Mutex<u64>,
        ask: Mutex<bool>,
    }

    impl MockHooks {
        fn new() -> Self {
            Self {
                prompts: Mutex::new(Vec::new()),
                activity_at: Mutex::new(0),
                ask: Mutex::new(false),
            }
        }
    }

    impl SubmitHooks for MockHooks {
        fn prompt_seen(&self, _agent_id: &str, since_ms: u64, tail: &str) -> bool {
            self.prompts
                .lock()
                .unwrap()
                .iter()
                .any(|(at, t)| *at >= since_ms && tail_matches(t, tail))
        }
        fn activity_seen(&self, _agent_id: &str, since_ms: u64) -> bool {
            let at = *self.activity_at.lock().unwrap();
            at > 0 && at >= since_ms
        }
        fn event_seen(&self, _agent_id: &str, since_ms: u64, events: &[&str]) -> bool {
            if events.contains(&"UserPromptSubmit") {
                self.prompts.lock().unwrap().iter().any(|(at, _)| *at >= since_ms)
            } else {
                false
            }
        }
        fn ask_open(&self, _agent_id: &str) -> bool {
            *self.ask.lock().unwrap()
        }
        fn compact_running_since(&self, _: &str, _: u64, _: u64, _: u64) -> u64 {
            0
        }
    }

    struct ScriptedProbe {
        replies: Mutex<Vec<TerminalProbeReply>>,
        calls: AtomicUsize,
    }

    impl ScriptedProbe {
        fn new(replies: Vec<TerminalProbeReply>) -> Self {
            Self {
                replies: Mutex::new(replies),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl ScreenProbe for ScriptedProbe {
        fn probe<'a>(
            &'a self,
            _pty_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = Option<TerminalProbeReply>> + Send + 'a>> {
            Box::pin(async move {
                let idx = self.calls.fetch_add(1, Ordering::SeqCst);
                let guard = self.replies.lock().unwrap();
                if guard.is_empty() {
                    None
                } else {
                    Some(guard[idx.min(guard.len() - 1)].clone())
                }
            })
        }
    }

    #[test]
    fn screen_parsers_detect_box_text_and_approval_prompts() {
        let lines = vec![
            "╭──────────────────────────────────╮".to_string(),
            "│ ❯ check git status               │".to_string(),
            "╰──────────────────────────────────╯".to_string(),
            "  Do you want to run git status?".to_string(),
            "  Enter to confirm or Esc to cancel".to_string(),
        ];
        assert_eq!(input_box_text(&lines).as_deref(), Some("check git status"));
        assert_eq!(submit_still_pending(&lines, "check git status"), Some(true));
        assert!(box_has_unsent_draft(&lines));
        assert!(prompt_on_screen(&lines));
    }

    #[tokio::test]
    async fn submit_verifies_via_hook_proof_and_screen_proof() {
        let mgr = PtyManager::new();
        mgr.spawn(SpawnOptions {
            id: "pty-a1".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "echo ready; sleep 5".into()],
            cwd: "/tmp".into(),
            cols: Some(80),
            rows: Some(24),
            env: HashMap::new(),
            provider: Some("claude".into()),
            resume: None,
            shell_script: None,
            hive: None,
        });
        let submitter = PtySubmitter::with_timings(mgr.clone(), SubmitTimings::fast_for_test());
        let hooks = Arc::new(MockHooks::new());
        let hooks_clone = hooks.clone();
        let res = submitter
            .submit_with_context(
                PtySubmitRequest {
                    pty_id: "pty-a1".into(),
                    agent_id: Some("a1".into()),
                    provider: "claude".into(),
                    text: "run cargo test now".into(),
                    source: "queue".into(),
                    settle_ms: None,
                    press_only: false,
                    idle: Some(true),
                    ref_id: Some("ref-1".into()),
                },
                None,
                hooks.as_ref(),
                move |_| {
                    let at = unix_now_ms() + 1;
                    hooks_clone
                        .prompts
                        .lock()
                        .unwrap()
                        .push((at, prompt_tail("run cargo test now")));
                    *hooks_clone.activity_at.lock().unwrap() = at;
                },
            )
            .await
            .unwrap();
        assert_eq!(res.outcome, "submitted");
        assert_eq!(res.proof, "hook");

        // Screen proof: seen in box, then cleared twice.
        let hooks_clean = MockHooks::new();
        let boxed = TerminalProbeReply {
            id: "p1".into(),
            lines: Some(vec![
                "────────────────────────────".into(),
                "❯ second prompt here".into(),
                "────────────────────────────".into(),
            ]),
            safe: true,
            pooled: false,
        };
        let cleared = TerminalProbeReply {
            id: "p2".into(),
            lines: Some(vec![
                "────────────────────────────".into(),
                "❯ ".into(),
                "────────────────────────────".into(),
            ]),
            safe: true,
            pooled: false,
        };
        let probe = ScriptedProbe::new(vec![
            cleared.clone(),
            boxed.clone(),
            boxed,
            cleared.clone(),
            cleared,
        ]);
        let res2 = submitter
            .submit_with_context(
                PtySubmitRequest {
                    pty_id: "pty-a1".into(),
                    agent_id: Some("a1".into()),
                    provider: "claude".into(),
                    text: "second prompt here".into(),
                    source: "queue".into(),
                    settle_ms: None,
                    press_only: false,
                    idle: Some(false),
                    ref_id: None,
                },
                Some(&probe),
                &hooks_clean,
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(res2.outcome, "submitted");
        assert_eq!(res2.proof, "screen");
        let _ = mgr.kill("pty-a1");
    }

    #[tokio::test]
    async fn submit_holds_when_prompt_on_screen() {
        let mgr = PtyManager::new();
        mgr.spawn(SpawnOptions {
            id: "pty-a2".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "echo ready; sleep 5".into()],
            cwd: "/tmp".into(),
            cols: Some(80),
            rows: Some(24),
            env: HashMap::new(),
            provider: Some("codex".into()),
            resume: None,
            shell_script: None,
            hive: None,
        });
        let submitter = PtySubmitter::with_timings(mgr.clone(), SubmitTimings::fast_for_test());
        let hooks = MockHooks::new();
        let ask_screen = TerminalProbeReply {
            id: "p1".into(),
            lines: Some(vec![
                "────────────────────────────".into(),
                "Would you like to run rm -rf?".into(),
                "Press enter to confirm or esc to cancel".into(),
            ]),
            safe: false,
            pooled: false,
        };
        let probe = ScriptedProbe::new(vec![ask_screen]);
        let res = submitter
            .submit_with_context(
                PtySubmitRequest {
                    pty_id: "pty-a2".into(),
                    agent_id: Some("a2".into()),
                    provider: "codex".into(),
                    text: "hello".into(),
                    source: "nudge".into(),
                    settle_ms: None,
                    press_only: false,
                    idle: Some(true),
                    ref_id: None,
                },
                Some(&probe),
                &hooks,
                |_| {},
            )
            .await
            .unwrap();
        assert_eq!(res.outcome, "held");
        assert!(res.behind);
        let _ = mgr.kill("pty-a2");
    }
}
