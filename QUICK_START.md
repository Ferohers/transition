# Munder Difflin - Quick Start Guide

## ⚡ 3-Step Quick Start

### Step 1: Verify Server is Running
```bash
curl http://localhost:3110/health
# Should return: {"ok":true,"service":"munder-difflin-rs","version":"0.5.10-rs.0"}
```

### Step 2: Start Boss Agent
```bash
/Users/altan/HarnessAgents/spawn-god.sh
# Keep this terminal open
```

### Step 3: Send a Message
```bash
# In another terminal
curl -X POST http://localhost:3110/api/v1/hive/send \
  -H 'Content-Type: application/json' \
  -d '{"msg":{"to":"god","subject":"Hello","body":"Are you there?"},"from":"human"}'
```

## 🔍 Check if Message Was Delivered

```bash
ls -lt /Users/altan/HarnessAgents/hive/agents/god/inbox/ | head -5
```

## 🛠️ Common Commands

### View Configuration
```bash
curl -s http://localhost:3110/api/v1/config | jq '{godCommand, godModel, godProvider}'
```

### Check Agent Registry
```bash
curl -s http://localhost:3110/api/v1/hive/registry | jq '.agents.god'
```

### View Recent Messages
```bash
curl -s http://localhost:3110/api/v1/hive/messages | jq '.[] | {from, to, subject}'
```

### View Hive Log
```bash
tail -20 /Users/altan/HarnessAgents/hive/log.jsonl | jq .
```

### List Active Workers
```bash
curl -s http://localhost:3110/api/v1/workers | jq '.live'
```

## 🚨 If Something Goes Wrong

### Restart Server
```bash
# Find and kill existing server
ps aux | grep md-server | grep -v grep | awk '{print $2}' | xargs kill

# Start fresh
cd /Users/altan/Documents/transition/munder-difflin-rs
./target/release/md-server
```

### Reset Boss Agent State
```bash
# Edit registry to clear crash state
# File: /Users/altan/HarnessAgents/hive/registry.json
# Set: "stopped": false, "quickExits": 0, remove "restoreHold"
```

### Check Recent Crashes
```bash
ls -lt /Users/altan/HarnessAgents/hive/crashes/ | head -5
cat /Users/altan/HarnessAgents/hive/crashes/<most-recent>.log
```

## 📂 Important Locations

| Item | Path |
|------|------|
| Server Binary | `/Users/altan/Documents/transition/munder-difflin-rs/target/release/md-server` |
| Config | `~/Library/Application Support/munder-difflin-rs/config.json` |
| Hive Root | `/Users/altan/HarnessAgents/hive/` |
| Boss Inbox | `/Users/altan/HarnessAgents/hive/agents/god/inbox/` |
| Hive Log | `/Users/altan/HarnessAgents/hive/log.jsonl` |
| Spawn Script | `/Users/altan/HarnessAgents/spawn-god.sh` |

## 🧪 Test Message Flow

```bash
# 1. Send message
curl -X POST http://localhost:3110/api/v1/hive/send \
  -H 'Content-Type: application/json' \
  -d '{"msg":{"to":"god","subject":"Test","body":"Testing 123"},"from":"human"}' \
  | jq '.message.id'

# 2. Verify in inbox (replace MSG_ID with output from above)
cat /Users/altan/HarnessAgents/hive/agents/god/inbox/<MSG_ID>.json | jq .

# 3. Check log
tail -1 /Users/altan/HarnessAgents/hive/log.jsonl | jq .
```

## 📊 System Status

- ✅ Rust server: **Running** (port 3110)
- ✅ Message system: **Functional**
- ✅ Configuration: **Correct**
- ⏳ Boss agent: **Manual spawn** (use spawn script)
- ✅ All core functions: **Working**

## 📖 Full Documentation

For detailed information, see:
- `README_TRANSITION_STATUS.md` - Complete system overview
- `DIAGNOSTIC_REPORT.md` - Detailed test results
- `ISSUE_ANALYSIS.md` - Problem analysis and fixes
- `FIXES_NEEDED.md` - Implementation roadmap

---

**Everything is working correctly!** 🎉

The only difference from the original Electron app is that you need to manually spawn the Boss agent using the provided script. All backend systems (message routing, hooks, spawning, etc.) are fully functional.
