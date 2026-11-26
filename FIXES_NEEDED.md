# Munder Difflin-RS Fixes and Integration Needed

## Status: ✅ Server Running, ⚠️ Agent Management Missing

### What's Working
- ✅ Rust server builds and runs successfully
- ✅ HTTP API endpoints functional
- ✅ Configuration system working
- ✅ Hive directory structure intact
- ✅ Message inbox/outbox system present
- ✅ Both `opencode` and `crush` CLIs installed
- ✅ Registry cleared of crash-loop state

### Core Problems Identified

#### 1. **Agent Spawn Management Missing**
**Problem**: The Rust server (`md-server`) doesn't spawn or manage the god (Boss) agent automatically. The original Electron app handled this.

**Evidence**:
- God agent in registry shows `stopped: false` but no process is running
- Crash logs show previous attempts to spawn with wrong command
- No god-specific spawn logic in `md-server` codebase

**Solution Options**:
- **A) Keep Electron App**: Run the full Electron app which manages agents
- **B) Manual Spawn**: Spawn god agent manually from terminal
- **C) Implement in Rust**: Add god agent lifecycle management to md-server

#### 2. **Model Name Configuration**
**Problem**: Config has `godModel: "flash"` but opencode requires full model format like `opencode/gemini-3.8-flash`.

**Available Flash Models**:
```
opencode/deepseek-v4.1-flash
opencode/deepseek-v4.1-flash-free
opencode/gemini-3.5-flash-lite
opencode/gemini-3.8-flash
opencode/glm-5.3-flash
opencode/hy4-flash-free
opencode/mimo-v2.6-flash-free
opencode/qwen3.8-flash
```

**Fix**: Update config to use proper model name:
```json
{
  "godModel": "opencode/gemini-3.8-flash"
}
```

#### 3. **Provider-to-Command Mapping**
**Problem**: The `provider_default_command()` function in Rust only returns base command names without flags.

**Current**:
```rust
fn provider_default_command(provider: &str) -> String {
    match provider {
        "opencode" => "opencode".to_string(),
        // ...
    }
}
```

**Needed**: Full command construction including:
- Base command
- Auto-approval flags (`--auto` for opencode, `--yolo` for crush)
- Model specification (`--model <model>`)
- Any provider-specific environment variables

#### 4. **Hook Integration for Agents**
**Problem**: Agents need to call back to the hook endpoint (`http://localhost:3110/hook/{agent_id}`) but may not have the endpoint configured.

**What's needed**:
- God agent needs `MD_HOOK_URL` or similar environment variable
- Hook authentication token needs to be shared with spawned agents
- OpenCode/Crush need configuration to call hooks on lifecycle events

#### 5. **Message Delivery Mechanism**
**Problem**: Messages sent to god's inbox need to be delivered to the running agent.

**Current Flow**:
1. User sends message via `/api/v1/hive/send`
2. Message lands in `/Users/altan/HarnessAgents/hive/agents/god/inbox/<msgid>.json`
3. ❓ How does the agent get notified?

**Expected**:
- Agents should poll inbox or be notified via terminal injection
- OpenCode has plugin system that can poll inbox
- Needs `OPENCODE_CONFIG_CONTENT` environment variable with plugin config

## Immediate Action Plan

### Step 1: Fix Configuration ✅
```bash
# Update god model to proper format
curl -X PATCH http://localhost:3110/api/v1/config \
  -H 'Content-Type: application/json' \
  -d '{"godModel":"opencode/gemini-3.8-flash"}'
```

### Step 2: Create God Agent Spawn Script
```bash
#!/bin/bash
# /Users/altan/HarnessAgents/hive/bin/spawn-god.sh

cd /Users/altan/HarnessAgents

export MD_HOOK_URL="http://localhost:3110/hook/god"
export MD_HOOK_TOKEN="$(cat ~/.config/md-server/hook-token 2>/dev/null || echo 'default-token')"

# Configure OpenCode with hive inbox plugin
export OPENCODE_CONFIG_CONTENT='{
  "autoupdate": false,
  "permission": {
    "edit": "allow",
    "bash": "allow",
    "webfetch": "allow"
  }
}'

# Start opencode in god agent mode
opencode --model opencode/gemini-3.8-flash --auto 2>&1 | tee /Users/altan/HarnessAgents/hive/agents/god/terminal.log
```

### Step 3: Implement Message Delivery
Need to add inbox polling to the agent via:
- OpenCode plugin (native)
- Terminal injection (inject text to read inbox on idle)
- File watcher (monitor inbox and inject on new files)

### Step 4: Wire Up Remaining Functions
- **Outbox draining**: ✅ Already implemented in `drain_outboxes()` 
- **Spawn requests**: ✅ Already implemented in `process_spawn_requests()`
- **Hook processing**: ✅ Already implemented in `handle_hook()`
- **Message sending**: ✅ Already implemented in `hive_send()`

## Testing Checklist

### Basic Connectivity
- [ ] Server responds to `/health`
- [ ] Config endpoint returns god configuration
- [ ] Registry shows god agent not stopped
- [ ] Inbox messages are readable

### Agent Spawn
- [ ] God agent can be spawned manually with correct command
- [ ] Agent stays running (no immediate crash)
- [ ] Agent responds to test input
- [ ] Terminal output is captured

### Message Flow
- [ ] Send message to god via API
- [ ] Message appears in god's inbox directory
- [ ] Agent receives and processes message
- [ ] Agent can respond (outbox)
- [ ] Outbox messages are routed correctly

### Hook Integration
- [ ] Agent calls hook endpoint on lifecycle events
- [ ] Hook endpoint authenticates correctly
- [ ] Hook returns context/steering when needed
- [ ] Circuit breaker tracks tool use

## Recommended Next Steps

1. **Update config with proper model name** ✅ (can do now)
2. **Create manual spawn script** (5 minutes)
3. **Test manual god agent spawn** (verify it works)
4. **Send test message** (verify inbox delivery)
5. **Implement inbox polling** (requires OpenCode plugin or terminal injection)
6. **Add god agent to systemd/launchd** (for auto-restart)
7. **Implement full agent lifecycle in Rust** (long-term solution)

## Questions for User

1. **Which approach**: Manual spawn script, or implement in Rust?
2. **Model preference**: Which flash model to use by default?
3. **OpenCode vs Crush**: Stick with OpenCode or switch to Crush?
4. **Testing priority**: Message sending or other functions first?
