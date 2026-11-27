//! Hook request processor mirroring `HookServer.handle` (`src/main/hookSink.ts`).

use crate::breaker::{BreakerState, CircuitBreaker};
use crate::control::ControlRegistry;
use crate::hook_proof::HookProofRecord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub agent_id: String,
    pub tokens: u64,
    pub limit: u64,
    pub ts: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSideEffects {
    pub events: Vec<(String, Value)>,
}

#[derive(Clone)]
pub struct HookEngine {
    pub proof: Arc<Mutex<HookProofRecord>>,
    pub control: Arc<Mutex<ControlRegistry>>,
    pub breaker: Arc<Mutex<CircuitBreaker>>,
    pub context_by_id: Arc<Mutex<HashMap<String, ContextSnapshot>>>,
    pub model_by_id: Arc<Mutex<HashMap<String, String>>>,
    pub session_by_id: Arc<Mutex<HashMap<String, String>>>,
    pub transcript_paths: Arc<Mutex<HashMap<String, String>>>,
    pub http_token: String,
}

pub fn normalize_hook_event(raw: &str) -> &str {
    match raw {
        "BeforeAgent" | "user_prompt_submit" => "UserPromptSubmit",
        "BeforeTool" | "pre_tool_use" => "PreToolUse",
        "AfterTool" | "post_tool_use" | "PostInvocation" => "PostToolUse",
        "post_tool_use_failure" => "PostToolUseFailure",
        "permission_denied" => "PermissionDenied",
        "AfterAgent" | "stop" => "Stop",
        "stop_failure" => "StopFailure",
        "session_start" | "sessionStart" => "SessionStart",
        "session_end" | "sessionEnd" => "SessionEnd",
        "notification" => "Notification",
        "subagent_start" => "SubagentStart",
        "subagent_stop" => "SubagentStop",
        "PreCompress" | "pre_compact" => "PreCompact",
        "post_compact" => "PostCompact",
        "PreInvocation" => "UserPromptSubmit",
        other => other,
    }
}

impl HookEngine {
    pub fn new(
        control: Arc<Mutex<ControlRegistry>>,
        breaker: Arc<Mutex<CircuitBreaker>>,
        http_token: String,
    ) -> Self {
        Self {
            proof: Arc::new(Mutex::new(HookProofRecord::new())),
            control,
            breaker,
            context_by_id: Arc::new(Mutex::new(HashMap::new())),
            model_by_id: Arc::new(Mutex::new(HashMap::new())),
            session_by_id: Arc::new(Mutex::new(HashMap::new())),
            transcript_paths: Arc::new(Mutex::new(HashMap::new())),
            http_token,
        }
    }

    pub fn clear_all(&self) {
        *self.proof.lock().unwrap() = HookProofRecord::new();
        self.context_by_id.lock().unwrap().clear();
        self.model_by_id.lock().unwrap().clear();
        self.session_by_id.lock().unwrap().clear();
        self.transcript_paths.lock().unwrap().clear();
    }

    pub fn forget_agent(&self, agent_id: &str) {
        self.proof.lock().unwrap().forget(agent_id);
        self.control.lock().unwrap().forget(agent_id);
        self.breaker.lock().unwrap().forget(agent_id);
        self.context_by_id.lock().unwrap().remove(agent_id);
        self.model_by_id.lock().unwrap().remove(agent_id);
        self.session_by_id.lock().unwrap().remove(agent_id);
        self.transcript_paths.lock().unwrap().remove(agent_id);
    }

    pub fn token_valid(&self, auth_header: Option<&str>) -> bool {
        let Some(header) = auth_header else {
            return false;
        };
        let expected = format!("Bearer {}", self.http_token);
        constant_time_eq(header.as_bytes(), expected.as_bytes())
    }

    pub fn handle_hook(
        &self,
        url_agent_id: &str,
        payload: &Value,
        standing_goal: Option<&str>,
        response_style: Option<&str>,
        session_identity: Option<&str>,
        roster_context: Option<&str>,
    ) -> (Value, HookSideEffects) {
        let mut side = HookSideEffects { events: Vec::new() };
        let agent_id = payload
            .get("agent_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(url_agent_id);
        let event = normalize_hook_event(
            payload
                .get("hook_event_name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown"),
        );
        let prompt = payload.get("prompt").and_then(|v| v.as_str());
        let source = payload.get("source").and_then(|v| v.as_str());
        let notification_type = payload.get("notification_type").and_then(|v| v.as_str());
        let message = payload.get("message").and_then(|v| v.as_str());
        let tool_name = payload.get("tool_name").and_then(|v| v.as_str());
        let tool_input = payload.get("tool_input");
        let now = now_ms();

        {
            let mut proof = self.proof.lock().unwrap();
            proof.note(Some(agent_id), event, prompt, message, tool_name, now);
        }

        if let Some(tp) = payload.get("transcript_path").and_then(|v| v.as_str())
            && !tp.is_empty()
        {
            self.transcript_paths
                .lock()
                .unwrap()
                .insert(agent_id.to_string(), tp.to_string());
        }
        if let Some(sid) = payload.get("session_id").and_then(|v| v.as_str())
            && !sid.is_empty()
        {
            self.session_by_id
                .lock()
                .unwrap()
                .insert(agent_id.to_string(), sid.to_string());
        }

        if let Some(model) = payload.get("model").and_then(|v| v.as_str())
            && !model.is_empty()
        {
            let mut models = self.model_by_id.lock().unwrap();
            if models.get(agent_id).map(|m| m.as_str()) != Some(model) {
                models.insert(agent_id.to_string(), model.to_string());
                side.events.push((
                    "hive:modelUpdate".to_string(),
                    serde_json::json!({ "agentId": agent_id, "model": model }),
                ));
            }
        }

        if event == "Status" {
            if let Some(cw) = payload.get("context_window")
                && let (Some(tokens), Some(limit)) = (
                    cw.get("total_input_tokens").and_then(|v| v.as_u64()),
                    cw.get("context_window_size").and_then(|v| v.as_u64()),
                )
                && limit > 0
            {
                let snap = ContextSnapshot {
                    agent_id: agent_id.to_string(),
                    tokens,
                    limit,
                    ts: now,
                };
                self.context_by_id
                    .lock()
                    .unwrap()
                    .insert(agent_id.to_string(), snap);
                side.events.push((
                    "hive:contextUpdate".to_string(),
                    serde_json::json!({ "agentId": agent_id, "tokens": tokens, "limit": limit }),
                ));
            }
            return (serde_json::json!({}), side);
        }

        let mut hook_ev = serde_json::json!({
            "agentId": agent_id,
            "event": event,
            "message": message,
            "tool": tool_name,
            "notificationType": notification_type,
            "source": source,
            "ts": now,
        });
        if event == "UserPromptSubmit"
            && let Some(p) = prompt
            && let Some(obj) = hook_ev.as_object_mut()
        {
            obj.insert(
                "promptTail".to_string(),
                Value::String(crate::hook_proof::prompt_tail(p)),
            );
        }
        side.events.push(("hive:hookEvent".to_string(), hook_ev));

        {
            let ctl = self.control.lock().unwrap();
            if ctl.should_halt(agent_id) {
                return (
                    serde_json::json!({
                        "continue": false,
                        "stopReason": "Halted by the operator from the floor."
                    }),
                    side,
                );
            }
        }

        // Feed CircuitBreaker
        let breaker_update: Option<BreakerState> = {
            let mut br = self.breaker.lock().unwrap();
            match event {
                "PostToolUse" => {
                    br.clear_error_streak(agent_id);
                    br.record_tool_use(agent_id, tool_name, tool_input, now)
                }
                "Stop" => {
                    br.clear_error_streak(agent_id);
                    None
                }
                "StopFailure" => br.record_error(agent_id, now),
                "UserPromptSubmit" => {
                    br.record_user_prompt(agent_id, now);
                    None
                }
                "PreCompact" => {
                    br.record_compact_start(agent_id, now);
                    None
                }
                "PostCompact" | "SessionStart" => {
                    br.record_compact_end(agent_id, now);
                    None
                }
                _ => None,
            }
        };
        if let Some(bs) = breaker_update
            && let Ok(val) = serde_json::to_value(bs)
        {
            side.events.push(("control:breakerState".to_string(), val));
        }

        if event == "PreToolUse" {
            let decision = {
                let ctl = self.control.lock().unwrap();
                ctl.tool_decision(agent_id, tool_name.unwrap_or(""))
            };
            if decision.deny {
                let reason = decision
                    .reason
                    .unwrap_or_else(|| "Denied by operator.".to_string());
                side.events.push((
                    "control:approvalRequest".to_string(),
                    serde_json::json!({
                        "agentId": agent_id,
                        "tool": tool_name,
                        "reason": reason,
                    }),
                ));
                return (
                    serde_json::json!({
                        "hookSpecificOutput": {
                            "hookEventName": "PreToolUse",
                            "permissionDecision": "deny",
                            "permissionDecisionReason": reason
                        }
                    }),
                    side,
                );
            }
        }

        let steer = if matches!(event, "UserPromptSubmit" | "PostToolUse") {
            let mut ctl = self.control.lock().unwrap();
            let s = ctl.take_steer(agent_id);
            if let Some(ref txt) = s {
                side.events.push((
                    "hive:steerLanded".to_string(),
                    serde_json::json!({
                        "agentId": agent_id,
                        "event": event,
                        "chars": txt.len(),
                        "tail": crate::hook_proof::prompt_tail(txt),
                    }),
                ));
            }
            s
        } else {
            None
        };

        let mut parts: Vec<&str> = Vec::new();
        if matches!(event, "SessionStart" | "UserPromptSubmit") {
            if let Some(st) = response_style.filter(|s| !s.is_empty()) {
                parts.push(st);
            }
            if let Some(id_ctx) = session_identity.filter(|s| !s.is_empty()) {
                parts.push(id_ctx);
            }
            if let Some(rc) = roster_context.filter(|s| !s.is_empty()) {
                parts.push(rc);
            }
            if let Some(g) = standing_goal.filter(|s| !s.is_empty()) {
                parts.push(g);
            }
        }
        if let Some(ref s) = steer {
            parts.push(s.as_str());
        }

        if !parts.is_empty() {
            return (
                serde_json::json!({
                    "hookSpecificOutput": {
                        "hookEventName": event,
                        "additionalContext": parts.join("\n\n")
                    }
                }),
                side,
            );
        }

        (serde_json::json!({}), side)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_tool_use_denied_when_paused_and_steer_ordered_last() {
        let ctl = Arc::new(Mutex::new(ControlRegistry::new()));
        let br = Arc::new(Mutex::new(CircuitBreaker::new()));
        let eng = HookEngine::new(ctl.clone(), br, "secret-token".to_string());

        assert!(eng.token_valid(Some("Bearer secret-token")));
        assert!(!eng.token_valid(Some("Bearer wrong")));

        ctl.lock().unwrap().pause("jim", true);
        let (reply, side) = eng.handle_hook(
            "jim",
            &serde_json::json!({ "hook_event_name": "PreToolUse", "tool_name": "Bash" }),
            None,
            None,
            None,
            None,
        );
        assert_eq!(
            reply["hookSpecificOutput"]["permissionDecision"],
            "deny"
        );
        assert!(side
            .events
            .iter()
            .any(|(ch, _)| ch == "control:approvalRequest"));

        ctl.lock().unwrap().resume("jim");
        ctl.lock().unwrap().steer("jim", "use cargo test");
        let (reply2, side2) = eng.handle_hook(
            "jim",
            &serde_json::json!({ "hook_event_name": "UserPromptSubmit", "prompt": "hello world" }),
            Some("<goal>ship</goal>"),
            Some("STYLE"),
            None,
            None,
        );
        let ctx = reply2["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert_eq!(ctx, "STYLE\n\n<goal>ship</goal>\n\nuse cargo test");
        assert!(side2.events.iter().any(|(ch, ev)| ch == "hive:hookEvent" && ev["promptTail"] == "helloworld"));
        assert!(side2.events.iter().any(|(ch, ev)| ch == "hive:steerLanded" && ev["tail"] == "usecargotest"));
    }
}
