# Munder Difflin Boss Agent Issue Analysis

## Problem Summary
The Boss (god) agent is crashing on startup with the error:
```
ERROR: Unknown flag: --model.
Try --help for usage.
```

## Root Cause
The agent is being spawned with the **crush** CLI but configured to use **opencode**:

### Current State
- **Config** (`~/Library/Application Support/munder-difflin-rs/config.json`):
  - `godCommand`: "opencode --auto"
  - `godProvider`: "opencode"
  - `godModel`: "flash"

- **Registry** (`/Users/altan/HarnessAgents/hive/registry.json`):
  - `provider`: "opencode"
  - `stopped`: true
  - `restoreHold`: "crash-loop"
  - `quickExits`: 3

- **Actual execution** (from crash log):
  - Command: `/opt/homebrew/bin/crush`
  - With flag: `--model` (which crush doesn't support)

### CLI Differences
- **OpenCode**: Supports `-m, --model provider/model` format
- **Crush**: Uses `--model provider/model-id` format BUT uses `--yolo` for auto-approval (not `--auto`)

## Issues Found

### 1. Command/Provider Mismatch
The agent is configured to use "opencode" but being executed with "crush". This could be from:
- Leftover state from previous Electron app
- Mismatch between config and actual spawn logic
- Agent being spawned from different code path

### 2. Model Flag Format
If crush is intended:
- Crush doesn't have `--auto` flag, it uses `--yolo`
- Both support `--model` but with slightly different semantics

### 3. Crash Loop Protection
The agent hit crash-loop protection after 3 quick exits, so it won't auto-restart.

## Immediate Fixes Needed

### Fix 1: Clear the Stopped State
```bash
# Stop server
kill $(ps aux | grep md-server | grep -v grep | awk '{print $2}')

# Remove crash-loop hold
# Option A: Edit registry.json to remove restoreHold and set stopped: false
# Option B: Delete god's agent state and let it recreate
```

### Fix 2: Ensure Consistent Provider Configuration
The godCommand in config should match the provider. Options:

**Option A: Use OpenCode (recommended based on config)**
```json
{
  "godCommand": "opencode --auto",
  "godProvider": "opencode",
  "godModel": "flash"
}
```

**Option B: Switch to Crush**
```json
{
  "godCommand": "crush --yolo",
  "godProvider": "crush",
  "godModel": "anthropic/claude-sonnet-4"
}
```

### Fix 3: Check Agent Spawn Logic
The Rust code needs to properly read `godCommand` from config when spawning the god agent. Currently in `routes_hive.rs`:

```rust
fn provider_default_command(provider: &str) -> String {
    match provider {
        "codex" => "codex".to_string(),
        "gemini" => "gemini".to_string(),
        "opencode" => "opencode".to_string(),
        "cursor" => "agent".to_string(),
        _ => "claude".to_string(),
    }
}
```

This returns the base command but doesn't include flags like `--auto` or model specification. Need to check where the actual spawn command is built for the god agent.

## Testing Steps

1. ✅ Server builds and runs
2. ✅ Config is readable and shows opencode configuration
3. ✅ Both opencode and crush CLIs are installed
4. ⏳ God agent spawn mechanism
5. ⏳ Message delivery to god agent
6. ⏳ Message processing and response

## Next Actions

1. **Clear crash-loop state** - Remove the `restoreHold` from registry
2. **Verify spawn code** - Find where god agent gets its command
3. **Test message send** - Send a test message to Boss
4. **Monitor logs** - Watch `/Users/altan/HarnessAgents/hive/log.jsonl` and crash logs
5. **Check other functions** - Review message routing, outbox draining, etc.
