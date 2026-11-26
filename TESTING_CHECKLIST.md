# Munder Difflin Testing Checklist

## ✅ Already Verified

- [x] Server builds in release mode
- [x] Server starts on port 3110
- [x] Health endpoint responds
- [x] Configuration loads correctly
- [x] Configuration updates work (PATCH)
- [x] God agent model updated to correct format
- [x] Registry cleared of crash-loop state
- [x] Message sending API works
- [x] Message lands in correct inbox directory
- [x] Message format is valid JSON
- [x] Both opencode and crush CLIs are installed
- [x] Hive directory structure intact
- [x] Spawn script created and made executable

## ⏳ Ready to Test (Requires Running Agent)

### Message Processing
- [ ] Start Boss agent with spawn script
- [ ] Verify agent doesn't crash immediately
- [ ] Send message to Boss via API
- [ ] Confirm Boss receives/processes message
- [ ] Boss sends response to outbox
- [ ] Outbox message is routed correctly
- [ ] Response appears in sender's inbox

### Hook Integration
- [ ] Agent calls hook endpoint on events
- [ ] Hook authentication succeeds
- [ ] Hook returns proper context
- [ ] Tool use is tracked by circuit breaker
- [ ] Steering messages are delivered

### Worker Management
- [ ] Create spawn-request file
- [ ] Server picks up spawn request
- [ ] Worker agent is created
- [ ] Worker receives initial objective
- [ ] Worker can communicate with god
- [ ] Worker completes task and sends "done"

### Task Management
- [ ] Create task via API
- [ ] Task appears in tasks.json
- [ ] Update task status
- [ ] Delete task
- [ ] Tasks survive server restart

### Additional Functions
- [ ] Board.md can be read/written
- [ ] Log entries are created correctly
- [ ] Crash logs are captured
- [ ] Registry updates persist
- [ ] PTY sessions work correctly
- [ ] Terminal resize works
- [ ] Command history is saved
- [ ] Integrations endpoints work
- [ ] File share functionality works
- [ ] Git integration functions

## 🧪 Test Commands

### Basic Connectivity
```bash
# Health
curl http://localhost:3110/health

# Config
curl -s http://localhost:3110/api/v1/config | jq .

# Registry
curl -s http://localhost:3110/api/v1/hive/registry | jq .
```

### Message Flow Test
```bash
# Send
curl -X POST http://localhost:3110/api/v1/hive/send \
  -H 'Content-Type: application/json' \
  -d '{"msg":{"to":"god","subject":"Test","body":"Hello"},"from":"human"}'

# Check inbox
ls -lt /Users/altan/HarnessAgents/hive/agents/god/inbox/ | head -3

# View message
cat /Users/altan/HarnessAgents/hive/agents/god/inbox/msg-*.json | jq .
```

### Worker Spawn Test
```bash
# Create request
cat > /Users/altan/HarnessAgents/hive/spawn-requests/test-worker.json << 'EOJ'
{
  "name": "Test Worker",
  "objective": "Print 'Worker spawned successfully' and exit",
  "cwd": "/Users/altan/HarnessAgents",
  "provider": "opencode"
}
EOJ

# Wait 1 second for server to process
sleep 1

# Check workers
curl -s http://localhost:3110/api/v1/workers | jq .
```

### Task Test
```bash
# Add task
curl -X POST http://localhost:3110/api/v1/hive/tasks \
  -H 'Content-Type: application/json' \
  -d '{"id":"TEST-1","title":"Integration test","status":"todo"}'

# List tasks
curl -s http://localhost:3110/api/v1/hive/tasks | jq '.tasks'

# Update task
curl -X PATCH http://localhost:3110/api/v1/hive/tasks/TEST-1 \
  -H 'Content-Type: application/json' \
  -d '{"status":"done"}'

# Delete task
curl -X DELETE http://localhost:3110/api/v1/hive/tasks/TEST-1
```

## 📋 Test Scenarios

### Scenario 1: Simple Message Exchange
1. Start server
2. Start Boss agent
3. Send "Hello" message to Boss
4. Verify Boss responds
5. Check response in human/god outbox/inbox

### Scenario 2: Worker Creation and Task
1. Create spawn request for worker
2. Wait for worker to spawn
3. Send task to worker
4. Worker executes task
5. Worker reports completion
6. Worker is archived

### Scenario 3: Multi-Agent Collaboration
1. Spawn 2 temp workers
2. God sends task to Worker1
3. Worker1 sends question to Worker2
4. Worker2 responds
5. Worker1 completes task
6. Worker1 reports to god

### Scenario 4: Circuit Breaker
1. Configure circuit breaker limits
2. Agent makes rapid tool calls
3. Circuit breaker triggers
4. Agent is paused
5. Resume agent
6. Normal operation resumes

## 🐛 Known Issues to Watch For

- [ ] Boss agent crashes with "Unknown flag" (should be fixed)
- [ ] Messages not delivered (agent not reading inbox)
- [ ] Spawn requests not processed (server poller issue)
- [ ] Hook authentication fails (token mismatch)
- [ ] PTY sessions don't capture output (terminal emulation)

## 📊 Success Criteria

**Minimum Viable**:
- Boss agent stays running
- Messages deliver to inbox
- Manual message acknowledgment works

**Full Functionality**:
- Automatic inbox polling
- Outbox draining works
- Multi-agent communication
- Worker spawning and lifecycle
- Hook integration active
- Circuit breaker functioning

## 📝 Notes

- Server must be running: `ps aux | grep md-server`
- Boss agent spawned: `ps aux | grep opencode`
- Logs available: `tail -f /Users/altan/HarnessAgents/hive/log.jsonl`
- All tests assume server on port 3110

---

Last Updated: 2024-11-26
Status: Ready for testing
Server: md-server v0.5.10-rs.0
