(() => {
  const listeners = new Map();
  const ptySockets = new Map();
  const recentEnqueues = new Map();

  // Scrub any stale/duplicated unread-inbox wake spam persisted in `cth.messageQueues`.
  try {
    const rawQueues = window.localStorage.getItem('cth.messageQueues');
    if (rawQueues) {
      const parsed = JSON.parse(rawQueues);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        let changed = false;
        for (const [aid, list] of Object.entries(parsed)) {
          if (!Array.isArray(list)) continue;
          const isWakeSpam = (txt) =>
            typeof txt === 'string' &&
            txt.startsWith('You have ') &&
            txt.includes('unread hive message(s) in ') &&
            txt.includes('inbox/.done');
          const hasStructuredWake = list.some(
            (item) => typeof item?.text === 'string' && item.text.startsWith('You have new hive inbox message(s)')
          );
          const wakeTexts = list
            .filter((item) => typeof item?.text === 'string' && item.text.startsWith('You have new hive inbox message(s)'))
            .map((item) => item.text);
          let keptOneWake = false;
          const cleaned = list.filter((item) => {
            const txt = item?.text;
            if (isWakeSpam(txt)) {
              changed = true;
              if (!keptOneWake && list.length <= 5) {
                keptOneWake = true;
                return true;
              }
              return false;
            }
            if (hasStructuredWake && typeof txt === 'string' && txt.startsWith('[HIVE ')) {
              const firstLine = txt.split('\n')[0] || '';
              const subj = firstLine.replace(/^\[HIVE [^\]]+\]\s*/, '').trim();
              if (subj && wakeTexts.some((wt) => wt.includes(subj))) {
                changed = true;
                return false;
              }
            }
            return true;
          });
          if (cleaned.length !== list.length) {
            parsed[aid] = cleaned;
            changed = true;
          }
        }
        if (changed) {
          window.localStorage.setItem('cth.messageQueues', JSON.stringify(parsed));
        }
      }
    }
  } catch {}

  function addListener(channel, fn) {
    if (typeof fn !== 'function') return () => {};
    if (!listeners.has(channel)) listeners.set(channel, new Set());
    listeners.get(channel).add(fn);
    return () => listeners.get(channel)?.delete(fn);
  }

  function normalizeEventChannel(channel) {
    if (channel === 'control:breakerState') return 'breakerState';
    if (channel === 'triggers:inbound' || channel === 'triggers:updated') return 'triggerHistoryUpdated';
    if (channel === 'hive:agentHold') return 'agentHold';
    const idx = channel.indexOf(':');
    if (idx <= 0) return channel;
    const prefix = channel.slice(0, idx);
    const rest = channel.slice(idx + 1);
    return prefix + rest.slice(0, 1).toUpperCase() + rest.slice(1);
  }

  function emitLocal(channel, payload) {
    const targets = [channel];
    const norm = normalizeEventChannel(channel);
    if (norm !== channel) targets.push(norm);
    for (const ch of targets) {
      const set = listeners.get(ch);
      if (!set) continue;
      for (const fn of set) {
        try { fn(payload); } catch (e) { console.error('[cth-web listener]', ch, e); }
      }
    }
  }

  async function req(url, opts = {}) {
    const res = await fetch(url, {
      headers: { 'content-type': 'application/json', ...(opts.headers || {}) },
      ...opts,
    });
    if (!res.ok) return null;
    const ct = res.headers.get('content-type') || '';
    return ct.includes('application/json') ? res.json() : res.text();
  }

  function syncGetJson(url, fallback) {
    try {
      const xhr = new XMLHttpRequest();
      xhr.open('GET', url, false);
      xhr.send(null);
      if (xhr.status >= 200 && xhr.status < 300 && xhr.responseText) {
        return JSON.parse(xhr.responseText);
      }
    } catch {}
    return fallback;
  }

  let notificationsEnabled = false;
  const agentNames = new Map();
  let agentNamesRefresh = null;

  function recordAgentEntry(entry) {
    if (!entry || typeof entry !== 'object') return;
    const rawId = typeof entry.id === 'string' && entry.id.trim() ? entry.id.trim() : '';
    const name = typeof entry.name === 'string' && entry.name.trim() ? entry.name.trim() : '';
    if (rawId && name) agentNames.set(rawId, name);
  }

  function recordAgentSnapshot(snap) {
    if (!snap || typeof snap !== 'object') return;
    for (const key of ['agents', 'active', 'archived', 'restorable']) {
      const val = snap[key];
      if (Array.isArray(val)) {
        for (const entry of val) recordAgentEntry(entry);
      } else if (val && typeof val === 'object') {
        for (const [id, entry] of Object.entries(val)) {
          if (entry && typeof entry === 'object') {
            recordAgentEntry({ id, ...entry });
          }
        }
      }
    }
  }

  function refreshAgentNames() {
    if (agentNamesRefresh) return agentNamesRefresh;
    agentNamesRefresh = Promise.all([
      req('/api/v1/roster').then(recordAgentSnapshot).catch(() => {}),
      req('/api/v1/hive/registry').then(recordAgentSnapshot).catch(() => {}),
    ]).finally(() => {
      agentNamesRefresh = null;
    });
    return agentNamesRefresh;
  }

  async function resolveAgentName(id, explicitName) {
    if (typeof explicitName === 'string' && explicitName.trim()) return explicitName.trim();
    if (typeof id !== 'string' || !id.trim()) return null;
    const trimmed = id.trim();
    const cached = agentNames.get(trimmed);
    if (cached) return cached;
    await refreshAgentNames();
    const refreshed = agentNames.get(trimmed);
    if (refreshed) return refreshed;
    return trimmed === 'god' ? 'Orchestrator' : trimmed;
  }

  function refreshNotificationSetting(cfg) {
    if (cfg && typeof cfg === 'object' && 'notifications' in cfg) {
      notificationsEnabled = cfg.notifications === true;
      if (notificationsEnabled && typeof Notification !== 'undefined' && Notification.permission === 'default') {
        Notification.requestPermission().catch(() => {});
      }
    }
  }

  function playAlertTone() {
    try {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      if (!Ctx) return;
      const ctx = new Ctx();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'sine';
      osc.frequency.value = 660;
      gain.gain.setValueAtTime(0.05, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + 0.18);
      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + 0.19);
      setTimeout(() => ctx.close().catch(() => {}), 250);
    } catch {}
  }

  async function dispatchUserNotification(channel, payload) {
    if (!notificationsEnabled || !payload) return;
    let title = null;
    let body = '';
    if (channel === 'hive:message' && (payload.to === 'human' || payload.needsHuman)) {
      const who = await resolveAgentName(payload.from, payload.fromName);
      title = `Message from ${who || 'agent'}`;
      body = payload.subject || payload.body || 'Needs your input';
    } else if (channel === 'control:approvalRequest') {
      const who = await resolveAgentName(payload.agentId, payload.agentName);
      title = `Approval needed (${who || 'agent'})`;
      body = payload.reason || payload.tool || 'Tool execution requires attention';
    } else if (channel === 'control:breakerState' && (payload.level === 'constrained' || payload.level === 'stopped')) {
      const who = await resolveAgentName(payload.agentId, payload.agentName);
      title = `Circuit breaker ${payload.level}: ${who || 'agent'}`;
      body = payload.reason || 'Agent paused by circuit breaker';
    } else if (channel === 'hive:hookEvent' && payload.event === 'Notification') {
      const msg = String(payload.message || '').toLowerCase();
      if (msg.includes('permission') || msg.includes('approve') || msg.includes('confirm') || msg.includes('needs your')) {
        const who = await resolveAgentName(payload.agentId, payload.agentName);
        title = `${who || 'Agent'} needs input`;
        body = String(payload.message || '');
      }
    } else if (channel === 'hive:agentExited' && (payload.exitCode !== 0 || String(payload.agentId || '').startsWith('worker-'))) {
      const who = await resolveAgentName(payload.agentId, payload.agentName);
      title = `${who || 'Agent'} finished (exit ${payload.exitCode ?? 0})`;
      body = payload.exitCode === 0 ? 'Task completed' : 'Exited with non-zero status';
    } else if (channel === 'triggers:inbound' && payload.pending) {
      title = 'Webhook trigger awaiting approval';
      body = payload.title || payload.endpointId || 'Review pending webhook';
    }
    if (!title) return;
    playAlertTone();
    if (typeof Notification !== 'undefined' && Notification.permission === 'granted') {
      try {
        const n = new Notification(title, { body: String(body).slice(0, 180), tag: `${channel}:${payload.agentId || payload.from || payload.id || ''}` });
        n.onclick = () => { try { window.focus(); n.close(); } catch {} };
      } catch {}
    }
  }

  function connectEventsWs() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${proto}//${location.host}/ws/events`);
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg && msg.channel) {
          if (msg.channel === 'config:changed') refreshNotificationSetting(msg.payload);
          if (msg.channel === 'app:reset' && !resettingLocally) {
            clearLocalStorageAndReload();
            return;
          }
          void dispatchUserNotification(msg.channel, msg.payload);
          // PTY data/exit events are delivered exclusively by the ordered per-PTY
          // socket (/ws/pty/:id) so chunks are never written to xterm.js twice.
          if (msg.channel.startsWith('pty:data:') || msg.channel.startsWith('pty:exit:')) return;
          if (msg.channel === 'hive:enqueue' && msg.payload) {
            if (msg.payload.fromInbox) return;
            const key = `${msg.payload.targetId || ''}:${msg.payload.text || ''}`;
            const now = Date.now();
            const prev = recentEnqueues.get(key) || 0;
            if (now - prev < 5000) return;
            recentEnqueues.set(key, now);
            if (recentEnqueues.size > 200) {
              for (const [k, t] of recentEnqueues) {
                if (now - t > 10000) recentEnqueues.delete(k);
              }
            }
          }
          emitLocal(msg.channel, msg.payload);
        }
      } catch {}
    };
    ws.onclose = () => setTimeout(connectEventsWs, 1500);
  }
  connectEventsWs();
  req('/api/v1/config').then(refreshNotificationSetting).catch(() => {});
  void refreshAgentNames();

  function ensurePtySocket(id) {
    if (!id || ptySockets.has(id)) return;
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${proto}//${location.host}/ws/pty/${encodeURIComponent(id)}`);
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        const kind = msg.kind || msg.type;
        if (kind === 'data') emitLocal(`pty:data:${id}`, msg.data);
        else if (kind === 'exit') emitLocal(`pty:exit:${id}`, msg);
      } catch {
        emitLocal(`pty:data:${id}`, ev.data);
      }
    };
    ws.onclose = () => ptySockets.delete(id);
    ptySockets.set(id, ws);
  }

  function buildTranscribeStatus(cfg) {
    const t = cfg && typeof cfg === 'object' ? cfg : {};
    return {
      platform: 'darwin',
      warming: false,
      chosen: { dictation: null, meeting: null },
      engines: { whisper: false, moonshine: false, groq: false, apple: false },
      model: {
        inUse: null,
        downloading: null,
        installed: { small: false, base: false, turbo: false },
        path: { small: '', base: '', turbo: '' },
      },
      moonshine: { helper: false, installed: false, path: '' },
      words: { total: 0, custom: Array.isArray(t.customWords) ? t.customWords.length : 0, whisperKept: 0 },
      config: { dictation: false, ...t },
    };
  }

  const ideChannel = typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel('cth-ide') : null;

  function arrayBufferToBase64(bytes) {
    const arr = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || 0);
    let bin = '';
    const chunk = 0x8000;
    for (let i = 0; i < arr.length; i += chunk) {
      bin += String.fromCharCode(...arr.subarray(i, i + chunk));
    }
    return btoa(bin);
  }

  function triggerDownload(filename, text) {
    const blob = new Blob([text], { type: 'application/json;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    a.style.display = 'none';
    document.body.appendChild(a);
    a.click();
    setTimeout(() => {
      a.remove();
      URL.revokeObjectURL(url);
    }, 200);
  }

  function pickLocalFiles({ accept = '*/*', multiple = false, readAs = 'text' } = {}) {
    return new Promise((resolve) => {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = accept;
      input.multiple = !!multiple;
      input.style.display = 'none';
      let settled = false;
      const finish = (val) => {
        if (settled) return;
        settled = true;
        window.removeEventListener('focus', onFocus);
        input.remove();
        resolve(val);
      };
      const onFocus = () => setTimeout(() => finish(null), 500);
      input.addEventListener('cancel', () => finish(null));
      input.addEventListener('change', async () => {
        const list = Array.from(input.files || []);
        if (!list.length) return finish(null);
        const out = [];
        for (const f of list) {
          if (readAs === 'text') {
            out.push({ name: f.name, size: f.size, text: await f.text() });
          } else {
            const buf = new Uint8Array(await f.arrayBuffer());
            out.push({ name: f.name, size: f.size, base64: arrayBufferToBase64(buf) });
          }
        }
        finish(out);
      });
      window.addEventListener('focus', onFocus, { once: true });
      document.body.appendChild(input);
      input.click();
    });
  }

  async function uploadRawBytes(name, bytes, scope = 'drops') {
    try {
      const base64 = arrayBufferToBase64(bytes);
      const res = await req('/api/v1/fs/upload', {
        method: 'POST',
        body: JSON.stringify({ name, base64, scope }),
      });
      return res || { ok: false, error: 'upload failed' };
    } catch (e) {
      return { ok: false, error: String(e) };
    }
  }

  const bootInfo = syncGetJson('/api/v1/info', null);
  let resetEpoch = typeof bootInfo?.resetEpoch === 'number' ? bootInfo.resetEpoch : null;
  let resettingLocally = false;

  function clearLocalStorageAndReload() {
    resettingLocally = true;
    try {
      const keys = [];
      for (let i = 0; i < window.localStorage.length; i++) {
        const k = window.localStorage.key(i);
        if (k && k.startsWith('cth.')) keys.push(k);
      }
      for (const k of keys) window.localStorage.removeItem(k);
    } catch {}
    setTimeout(() => window.location.reload(), 200);
  }

  const baseApi = {
    platform: bootInfo?.platform || (/Linux/i.test(navigator.platform || navigator.userAgent) ? 'linux' : /Win/i.test(navigator.platform || '') ? 'win32' : 'darwin'),
    arch: bootInfo?.arch || (/x86_64|Win64|x64|amd64/i.test(navigator.userAgent || navigator.platform || '') ? 'x64' : 'arm64'),
    version: bootInfo?.version || '0.5.10',
    windowChrome: {
      setControls: async () => {},
      popupMenu: async () => {},
    },
    appInfo: () => req('/api/v1/info'),
    usageNoticeNeeded: () => Promise.resolve(false),
    usageAcceptNotice: () => Promise.resolve(true),
    npsDue: () => Promise.resolve(false),
    npsAnswer: () => Promise.resolve(true),
    npsDismiss: () => Promise.resolve(true),
    windowOfficeSync: () => null,
    harnessHomeSync: () => syncGetJson('/api/v1/config', {})?.harnessHome ?? null,
    getConfig: () => req('/api/v1/config'),
    updateConfig: (patch) => req('/api/v1/config', { method: 'PATCH', body: JSON.stringify(patch || {}) }).then((c) => {
      if (c && !c.error) {
        refreshNotificationSetting(c);
        emitLocal('configChanged', c);
      }
      return c;
    }),
    ensureHarnessHome: (path) => req('/api/v1/config/ensure-home', { method: 'POST', body: JSON.stringify({ path }) }).then((r) => r || { ok: false, error: 'request failed' }),
    integrityIssues: () => req('/api/v1/integrity/issues').then((r) => (Array.isArray(r) ? r : [])),
    rosterReadSync: () => syncGetJson('/api/v1/roster', null),
    rosterWrite: async (snap) => {
      if (resettingLocally) return { ok: false, error: 'resetting' };
      recordAgentSnapshot(snap);
      const headers = { 'content-type': 'application/json' };
      if (resetEpoch !== null) headers['x-md-epoch'] = String(resetEpoch);
      try {
        const res = await fetch('/api/v1/roster', {
          method: 'PUT',
          headers,
          body: JSON.stringify(snap),
        });
        if (res.status === 409) {
          clearLocalStorageAndReload();
          return { ok: false, error: 'stale-session' };
        }
        if (!res.ok) return { ok: false, error: 'request failed' };
        return await res.json();
      } catch {
        return { ok: false, error: 'request failed' };
      }
    },
    historyAdd: (entry) => req('/api/v1/history', { method: 'POST', body: JSON.stringify(entry) }).then((r) => r || { ok: false }),
    historyList: (agentId, limit) => req(`/api/v1/history?${new URLSearchParams({ ...(agentId ? { agentId } : {}), ...(limit ? { limit: String(limit) } : {}) })}`).then((r) => (Array.isArray(r) ? r : [])),
    historySearch: (q, limit) => req(`/api/v1/history?${new URLSearchParams({ q, ...(limit ? { limit: String(limit) } : {}) })}`).then((r) => (Array.isArray(r) ? r : [])),

    // PTY
    spawnPty: async (opts) => {
      if (opts?.hive) recordAgentEntry(opts.hive);
      const res = await req('/api/v1/pty/spawn', { method: 'POST', body: JSON.stringify(opts) });
      return res || { ok: false, error: 'spawn failed' };
    },
    sleepScreen: (id) => req(`/api/v1/pty/${encodeURIComponent(id)}/sleep-screen`).then((r) => (typeof r === 'string' ? r : (r?.screen ?? ''))),
    writePty: (id, data) => req('/api/v1/pty/write', { method: 'POST', body: JSON.stringify({ id, data }) }).then((r) => r || { ok: false }),
    resizePty: (id, cols, rows) => req('/api/v1/pty/resize', { method: 'POST', body: JSON.stringify({ id, cols, rows }) }).then((r) => r || { ok: false }),
    redrawPty: (id) => req('/api/v1/pty/redraw', { method: 'POST', body: JSON.stringify({ id }) }).then((r) => r || { ok: false }),
    killPty: (id) => req('/api/v1/pty/kill', { method: 'POST', body: JSON.stringify({ id }) }).then((r) => r || { ok: false, error: 'kill failed' }),
    listPtys: () => req('/api/v1/pty/list').then((r) => (Array.isArray(r) ? r : [])),
    submitToPty: (payload) => req('/api/v1/pty/submit', { method: 'POST', body: JSON.stringify(payload) }).then((r) => (
      r && !r.result
        ? { ok: !!r.ok, sent: !!(r.ok && r.outcome === 'submitted'), error: r.error, result: r }
        : (r || { ok: false, error: 'submit failed' })
    )),
    withdrawPtySubmit: (ref) => req('/api/v1/pty/withdraw', { method: 'POST', body: JSON.stringify({ ref }) }).then((r) => r || { ok: false }),
    onPtyBegan: (fn) => addListener('pty:began', (v) => {
      const ptyId = typeof v === 'string' ? v : v?.ptyId;
      if (typeof ptyId === 'string' && typeof fn === 'function') fn(ptyId);
    }),
    onTerminalProbe: (fn) => addListener('terminal:probe', (probeReq) => {
      if (!probeReq || typeof fn !== 'function') return;
      let out = null;
      try { out = fn(probeReq); } catch { out = null; }
      const reply = { id: probeReq.id, ...(out || { lines: null, safe: false, pooled: true }) };
      void req('/api/v1/terminal/probe-reply', { method: 'POST', body: JSON.stringify(reply) }).catch(() => {});
    }),
    onPtyData: (id, fn) => { ensurePtySocket(id); return addListener(`pty:data:${id}`, fn); },
    onPtyExit: (id, fn) => { ensurePtySocket(id); return addListener(`pty:exit:${id}`, fn); },
    codexAllowedFolders: () => req('/api/v1/codex/allowed-folders').then((r) => (Array.isArray(r) ? r : [])),
    codexAllowFolder: (folder) => req('/api/v1/codex/allow-folder', { method: 'POST', body: JSON.stringify({ folder }) }).then((r) => (
      Array.isArray(r) ? { ok: true, codexFolders: r } : (r || { ok: false, error: 'request failed' })
    )),
    codexWithdrawFolder: (folder) => req('/api/v1/codex/withdraw-folder', { method: 'POST', body: JSON.stringify({ folder }) }).then((r) => (Array.isArray(r) ? r : [])),
    codexAllowWaitingFolder: () => Promise.resolve({ ok: false, error: 'not-waiting' }),

    // Hive & Controls
    hiveRegistry: () => req('/api/v1/hive/registry'),
    hiveTasks: () => req('/api/v1/hive/tasks').then((r) => (r && Array.isArray(r.tasks) ? r : { tasks: Array.isArray(r) ? r : [] })),
    hiveTasksArchive: () => req('/api/v1/hive/tasks/archive').then((r) => (r && Array.isArray(r.tasks) ? r : { tasks: Array.isArray(r) ? r : [] })),
    hiveAddTask: (task) => req('/api/v1/hive/tasks', { method: 'POST', body: JSON.stringify(task) }).then((r) => r || { ok: false, error: 'request failed' }),
    hivePatchTask: (id, patch) => req(`/api/v1/hive/tasks/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify(patch) }).then((r) => r || { ok: false, error: 'request failed' }),
    hiveDeleteTask: (id) => req(`/api/v1/hive/tasks/${encodeURIComponent(id)}`, { method: 'DELETE' }).then((r) => r || { ok: false, error: 'request failed' }),
    hiveBoard: () => req('/api/v1/hive/board').then((r) => (typeof r === 'string' ? r : (r?.board ?? ''))),
    hiveLog: (limit) => req(`/api/v1/hive/log${limit ? `?n=${limit}` : ''}`).then((r) => (Array.isArray(r) ? r : [])),
    hiveSend: (msg, from = 'human') => req('/api/v1/hive/send', { method: 'POST', body: JSON.stringify({ msg, from }) }).then((r) => r || { ok: false, error: 'request failed' }),
    hiveInbox: (agentId) => req(`/api/v1/hive/inbox/${encodeURIComponent(agentId)}`).then((r) => (Array.isArray(r) ? r : [])),
    hiveMessages: (arg) => {
      const params = new URLSearchParams();
      if (typeof arg === 'string') params.set('agentId', arg);
      else if (arg && typeof arg === 'object') {
        if (arg.agentId) params.set('agentId', arg.agentId);
        if (arg.limit) params.set('limit', String(arg.limit));
      }
      const qs = params.toString();
      return req(`/api/v1/hive/messages${qs ? `?${qs}` : ''}`).then((r) => (Array.isArray(r) ? r : []));
    },
    hiveHumanMailSince: (since) => req('/api/v1/hive/human-mail-since', { method: 'POST', body: JSON.stringify(since && typeof since === 'object' ? since : {}) }).then((r) => (r && typeof r === 'object' ? r : {})),
    hiveAgentDirectory: () => req('/api/v1/hive/directory').then((r) => (r && Array.isArray(r.agents) ? r : { godId: null, agents: [] })),
    hiveInboxPath: async (agentId) => {
      const cfg = await req('/api/v1/config').catch(() => null);
      return cfg?.harnessHome && agentId ? `${cfg.harnessHome}/hive/agents/${agentId}/inbox` : null;
    },
    hiveTicketPrefix: () => req('/api/v1/config').then((c) => (typeof c?.ticketPrefix === 'string' ? c.ticketPrefix : '')).catch(() => ''),
    getContextTrigger: () => req('/api/v1/context-trigger').then((r) => (r && typeof r === 'object' ? r : {})),
    setContextTrigger: (cfg) => req('/api/v1/context-trigger', { method: 'POST', body: JSON.stringify(cfg || {}) }).then((r) => r || { ok: false }),
    onContextTrigger: (fn) => addListener('context:trigger', fn),
    historyPage: async (arg = {}) => {
      const params = new URLSearchParams();
      if (typeof arg === 'string') params.set('agentId', arg);
      else if (arg && typeof arg === 'object') {
        if (arg.agentId) params.set('agentId', arg.agentId);
        if (arg.limit) params.set('limit', String(arg.limit));
        if (typeof arg.after === 'number') params.set('after', String(arg.after));
        if (typeof arg.before === 'number') params.set('before', String(arg.before));
      }
      const qs = params.toString();
      const res = await req(`/api/v1/history/page${qs ? `?${qs}` : ''}`).catch(() => null);
      return res && res.ok ? res : { ok: true, fileId: 'empty', start: 0, end: 0, atStart: true, live: true, items: [], hasMore: false };
    },
    textSearch: (q) => req(`/api/v1/hive/search?q=${encodeURIComponent(q)}`).then((r) => {
      if (r && typeof r.ok === 'boolean') return r;
      if (!Array.isArray(r)) return { ok: false, results: [], error: 'search failed' };
      return {
        ok: true,
        results: r.map((h) => ({
          source: h.source ?? h.file ?? '',
          line: h.line ?? 0,
          excerpt: h.excerpt ?? h.snippet ?? '',
        })),
      };
    }),
    controlSnapshot: (agentId) => req(`/api/v1/control/${encodeURIComponent(agentId || 'god')}/snapshot`),
    controlPause: (agentId, on = true) => req('/api/v1/control/pause', { method: 'POST', body: JSON.stringify({ agentId, on: typeof on === 'boolean' ? on : true }) }).then((r) => r?.snapshot ?? null),
    controlAutoDelivery: (agentId, paused = true) => req('/api/v1/control/auto-delivery', { method: 'POST', body: JSON.stringify({ agentId, on: !!paused }) }).then((r) => r?.snapshot ?? null),
    controlResume: (agentId) => req('/api/v1/control/resume', { method: 'POST', body: JSON.stringify({ agentId }) }).then((r) => r?.snapshot ?? null),
    controlSteer: (agentId, text) => req('/api/v1/control/steer', { method: 'POST', body: JSON.stringify({ agentId, text }) }).then((r) => r?.snapshot ?? null),
    controlHalt: (agentId) => req('/api/v1/control/halt', { method: 'POST', body: JSON.stringify({ agentId: agentId || 'god' }) }).then((r) => r?.snapshot ?? null),
    controlUnhalt: (agentId) => req('/api/v1/control/unhalt', { method: 'POST', body: JSON.stringify({ agentId: agentId || 'god' }) }).then((r) => r?.snapshot ?? null),
    breakerSnapshot: () => req('/api/v1/breaker/snapshot').then((r) => (Array.isArray(r) ? r : [])).catch(() => []),
    breakerSetState: (state) => req('/api/v1/breaker/state', { method: 'POST', body: JSON.stringify(state || {}) }).then((r) => r || { ok: false }),

    // FS & Git
    listDir: (root, rel = '') => req('/api/v1/fs/list', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    readFile: (root, rel) => req('/api/v1/fs/read', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    readBinary: async (root, rel) => {
      const r = await req('/api/v1/fs/read-binary', { method: 'POST', body: JSON.stringify({ root, rel }) });
      if (!r || !r.ok) return r || { ok: false, error: 'read failed' };
      if (typeof r.base64 === 'string' && !(r.bytes instanceof Uint8Array)) {
        const bin = atob(r.base64);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        return { ...r, bytes };
      }
      return r;
    },
    writeFile: (root, rel, content) => req('/api/v1/fs/write', { method: 'POST', body: JSON.stringify({ root, rel, content }) }),
    searchFiles: (root, query, opts) => req('/api/v1/fs/search', { method: 'POST', body: JSON.stringify({ root, query, opts }) }),
    makeDir: (root, rel) => req('/api/v1/fs/mkdir', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    createFile: (root, rel) => req('/api/v1/fs/create', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    renamePath: (root, from, to) => req('/api/v1/fs/rename', { method: 'POST', body: JSON.stringify({ root, from, to }) }),
    trashPath: (root, rel) => req('/api/v1/fs/trash', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    statAbs: (path) => req('/api/v1/fs/stat-abs', { method: 'POST', body: JSON.stringify({ path }) }),
    ideDirty: (unsaved, openFiles) => req('/api/v1/ide/dirty', { method: 'POST', body: JSON.stringify({ unsaved: unsaved || 0, openFiles: Array.isArray(openFiles) ? openFiles : [] }) }),
    ideHandoff: async () => {
      const [ide, roster] = await Promise.all([
        req('/api/v1/ide/state').catch(() => null),
        req('/api/v1/roster').catch(() => null),
      ]);
      const openFiles = Array.isArray(ide?.openFiles) ? ide.openFiles : [];
      const handoff = openFiles.length > 0 ? { path: openFiles[0] } : null;
      const agents = Array.isArray(roster?.agents) ? roster.agents : [];
      return { handoff, agents };
    },
    ideOwnerOffice: () => Promise.resolve(null),
    ideCloseWithoutSaving: () => Promise.resolve({ ok: true }),
    chooseFolder: async () => {
      const data = await req('/api/v1/workspaces/list').catch(() => null);
      const folders = Array.isArray(data?.folders) ? data.folders : [];
      const home = data?.harnessHome || '~/HarnessAgents';
      return new Promise((resolve) => {
        const overlay = document.createElement('div');
        overlay.style.cssText = 'position:fixed;inset:0;z-index:999999;background:rgba(12,12,16,0.68);display:flex;align-items:center;justify-content:center;padding:16px;font-family:var(--cth-font-ui,system-ui,sans-serif);';
        const card = document.createElement('div');
        card.style.cssText = 'width:min(520px,100%);background:var(--cth-paper-100,#f6f4ef);color:var(--cth-ink-900,#1d1b18);border:1px solid var(--cth-ink-300,#c8c2b6);border-radius:10px;padding:16px;box-shadow:0 20px 50px rgba(0,0,0,0.35);display:flex;flex-direction:column;gap:12px;';

        const title = document.createElement('div');
        title.textContent = 'Select or Create Workspace Folder';
        title.style.cssText = 'font-weight:600;font-size:14px;';
        card.appendChild(title);

        const hint = document.createElement('div');
        hint.textContent = `Pick an existing folder, or type a new subfolder name (created inside ${home}):`;
        hint.style.cssText = 'font-size:12px;opacity:0.78;line-height:1.4;';
        card.appendChild(hint);

        const input = document.createElement('input');
        input.type = 'text';
        input.value = home;
        input.placeholder = `${home}/projects/my-project or my-project`;
        input.style.cssText = 'width:100%;box-sizing:border-box;padding:8px 10px;border-radius:6px;border:1px solid var(--cth-ink-300,#c8c2b6);background:var(--cth-cream-100,#fff);color:inherit;font-family:var(--cth-font-mono,monospace);font-size:12px;';
        card.appendChild(input);

        if (folders.length > 0) {
          const listLabel = document.createElement('div');
          listLabel.textContent = 'Available workspaces';
          listLabel.style.cssText = 'font-size:11px;font-weight:600;text-transform:uppercase;letter-spacing:0.04em;opacity:0.65;';
          card.appendChild(listLabel);

          const list = document.createElement('div');
          list.style.cssText = 'max-height:180px;overflow-y:auto;display:flex;flex-direction:column;gap:4px;';
          for (const f of folders) {
            const btn = document.createElement('button');
            btn.type = 'button';
            btn.textContent = f;
            btn.style.cssText = 'text-align:left;padding:6px 8px;border-radius:6px;border:1px solid var(--cth-ink-300,#d5cfc3);background:var(--cth-cream-100,#fff);color:inherit;font-family:var(--cth-font-mono,monospace);font-size:11.5px;cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;';
            btn.onclick = () => {
              input.value = f;
              void submit();
            };
            list.appendChild(btn);
          }
          card.appendChild(list);
        }

        const errEl = document.createElement('div');
        errEl.style.cssText = 'font-size:12px;color:#b42318;display:none;';
        card.appendChild(errEl);

        const actions = document.createElement('div');
        actions.style.cssText = 'display:flex;justify-content:flex-end;gap:8px;margin-top:4px;';

        const cancelBtn = document.createElement('button');
        cancelBtn.type = 'button';
        cancelBtn.textContent = 'Cancel';
        cancelBtn.style.cssText = 'padding:6px 12px;border-radius:6px;border:1px solid var(--cth-ink-300,#c8c2b6);background:transparent;color:inherit;cursor:pointer;font-size:12px;';

        const okBtn = document.createElement('button');
        okBtn.type = 'button';
        okBtn.textContent = 'Use / Create Folder';
        okBtn.style.cssText = 'padding:6px 12px;border-radius:6px;border:none;background:var(--cth-ink-900,#1d1b18);color:var(--cth-cream-100,#fff);cursor:pointer;font-size:12px;font-weight:500;';

        let settled = false;
        const done = (res) => {
          if (settled) return;
          settled = true;
          window.removeEventListener('keydown', onKey, true);
          overlay.remove();
          resolve(res);
        };
        const submit = async () => {
          const val = input.value.trim();
          if (!val) {
            errEl.textContent = 'Enter a folder name or path.';
            errEl.style.display = 'block';
            return;
          }
          okBtn.disabled = true;
          const ensured = await req('/api/v1/workspaces/ensure', {
            method: 'POST',
            body: JSON.stringify({ path: val }),
          }).catch(() => null);
          okBtn.disabled = false;
          if (ensured?.ok && ensured.path) {
            done({ ok: true, path: ensured.path });
          } else {
            errEl.textContent = ensured?.error || 'Could not resolve folder path.';
            errEl.style.display = 'block';
          }
        };
        const onKey = (ev) => {
          if (ev.key === 'Escape') {
            ev.preventDefault();
            ev.stopPropagation();
            done({ ok: false, error: 'cancelled' });
          } else if (ev.key === 'Enter' && document.activeElement === input) {
            ev.preventDefault();
            void submit();
          }
        };
        cancelBtn.onclick = () => done({ ok: false, error: 'cancelled' });
        okBtn.onclick = () => void submit();
        overlay.onclick = (ev) => {
          if (ev.target === overlay) done({ ok: false, error: 'cancelled' });
        };
        window.addEventListener('keydown', onKey, true);

        actions.appendChild(cancelBtn);
        actions.appendChild(okBtn);
        card.appendChild(actions);
        overlay.appendChild(card);
        document.body.appendChild(overlay);
        input.focus();
        input.select();
      });
    },
    onCompactAsked: (fn) => addListener('hive:compactAsked', fn),
    toolsStatus: () => req('/api/v1/tools/status'),
    hiveTheme: () => req('/api/v1/config').then((c) => c?.officeTheme ?? null),
    setHiveTheme: (_home, officeTheme) => req('/api/v1/config', { method: 'PATCH', body: JSON.stringify({ officeTheme }) }).then((c) => ({ ok: !!c })),
    teamsSetBossName: async (name) => {
      const trimmed = typeof name === 'string' ? name.trim() : '';
      if (!trimmed) return { ok: false, error: 'invalid', detail: 'Name required' };
      agentNames.set('god', trimmed);
      const updated = await req('/api/v1/config', {
        method: 'PATCH',
        body: JSON.stringify({ godName: trimmed }),
      });
      await req('/api/v1/hive/agent/rename', {
        method: 'POST',
        body: JSON.stringify({ id: 'god', value: trimmed }),
      }).catch(() => null);
      return updated ? { ok: true, saved: true } : { ok: false, error: 'failed' };
    },
    teamsSetName: () => Promise.resolve({ ok: true, offline: true }),
    hiveRenameAgent: (id, value) => req('/api/v1/hive/agent/rename', { method: 'POST', body: JSON.stringify({ id, value }) }).then((r) => r || { ok: false }),
    hivePatchAgentRole: (id, value) => req('/api/v1/hive/agent/role', { method: 'POST', body: JSON.stringify({ id, value }) }).then((r) => r || { ok: false }),
    hivePatchAgentGoal: (id, value) => req('/api/v1/hive/agent/goal', { method: 'POST', body: JSON.stringify({ id, value }) }).then((r) => r || { ok: false }),
    hivePatchAgentLook: () => Promise.resolve({ ok: true }),
    hiveSetAgentHold: (id, value) => req('/api/v1/hive/agent/hold', { method: 'POST', body: JSON.stringify({ id, value: !!value }) }).then((r) => r || { ok: false }),
    hiveSetArchived: (id, value) => req('/api/v1/hive/agent/archived', { method: 'POST', body: JSON.stringify({ id, value: !!value }) }).then((r) => r || { ok: false }),
    reportFloorState: () => Promise.resolve({ ok: true }),
    cliMissingState: () => Promise.resolve(null),
    cliSetupState: () => Promise.resolve(null),
    codexHeldFolders: () => Promise.resolve(null),
    codexWaitingFolders: () => Promise.resolve([]),
    gitIsRepo: (cwd) => req(`/api/v1/git/is-repo?cwd=${encodeURIComponent(cwd)}`),
    gitMainRepo: (cwd) => req(`/api/v1/git/main-repo?cwd=${encodeURIComponent(cwd)}`),
    gitBranch: (cwd) => req(`/api/v1/git/branch?cwd=${encodeURIComponent(cwd)}`),
    gitStatus: (cwd) => req(`/api/v1/git/status?cwd=${encodeURIComponent(cwd)}`),
    gitLog: (cwd, n) => req(`/api/v1/git/log?cwd=${encodeURIComponent(cwd)}${n ? `&n=${n}` : ''}`),
    gitBranches: (cwd) => req(`/api/v1/git/branches?cwd=${encodeURIComponent(cwd)}`),
    gitAheadBehind: (cwd) => req(`/api/v1/git/ahead-behind?cwd=${encodeURIComponent(cwd)}`),
    gitDiff: (cwd, relPath) => req(`/api/v1/git/diff?cwd=${encodeURIComponent(cwd)}&relPath=${encodeURIComponent(relPath)}`),
    gitWorktrees: (cwd) => req(`/api/v1/git/worktrees?cwd=${encodeURIComponent(cwd)}`),
    gitCheckout: (cwd, ref, detach = false) => req('/api/v1/git/checkout', { method: 'POST', body: JSON.stringify({ cwd, ref, detach: !!detach }) }),
    gitLogGraph: (cwd, n = 200, skip = 0) => req(`/api/v1/git/log-graph?cwd=${encodeURIComponent(cwd)}&n=${n}&skip=${skip}`).then((r) => r || { error: 'request failed' }),
    gitCommitFiles: (cwd, sha) => req(`/api/v1/git/commit-files?cwd=${encodeURIComponent(cwd)}&sha=${encodeURIComponent(sha)}`).then((r) => r || { error: 'request failed' }),
    gitShowFile: (cwd, rev, relPath) => req(`/api/v1/git/show-file?cwd=${encodeURIComponent(cwd)}&rev=${encodeURIComponent(rev)}&relPath=${encodeURIComponent(relPath)}`).then((r) => r || { ok: false, error: 'request failed' }),
    gitCompareRefs: (cwd, base, head, mode = 'three') => req(`/api/v1/git/compare-refs?cwd=${encodeURIComponent(cwd)}&base=${encodeURIComponent(base)}&head=${encodeURIComponent(head)}&mode=${encodeURIComponent(mode)}`).then((r) => r || { error: 'request failed' }),
    listOwnedWorktrees: () => req('/api/v1/worktrees/list').then((r) => (Array.isArray(r) ? r : [])),
    removeOwnedWorktree: (path, confirmed = false) => req('/api/v1/worktrees/remove', { method: 'POST', body: JSON.stringify({ path, confirmed: !!confirmed }) }).then((r) => r || { ok: false, error: 'request failed' }),
    worktreeWork: (path, baseBranch) => req('/api/v1/worktrees/work', { method: 'POST', body: JSON.stringify({ path, baseBranch }) }).then((r) => r || { known: false, uncommitted: 0, unmerged: 0, base: baseBranch || 'main' }),
    folderState: (path) => req('/api/v1/fs/folder-state', { method: 'POST', body: JSON.stringify({ path }) }).then((r) => r || { exists: false, empty: true, path }),
    floorProbe: async (paths) => {
      const list = Array.isArray(paths) ? paths : [];
      const cfg = await req('/api/v1/config').catch(() => null);
      const currentHome = String(cfg?.harnessHome || '').replace(/\/+$/, '');
      return Promise.all(list.map(async (p) => {
        const norm = String(p || '').replace(/\/+$/, '');
        const st = await req('/api/v1/fs/stat-abs', { method: 'POST', body: JSON.stringify({ path: p }) }).catch(() => null);
        const state = norm && currentHome && norm === currentHome ? 'current' : st?.isDir ? 'free' : 'missing';
        return { path: p, exists: !!st?.exists, isDir: !!st?.isDir, state, lock: null };
      }));
    },
    floorOpen: async ({ harnessHome } = {}) => {
      if (!harnessHome) return { ok: false, refusal: 'invalid', lock: null };
      const res = await req('/api/v1/floor/switch', { method: 'POST', body: JSON.stringify({ harnessHome }) });
      if (!res?.ok) return res || { ok: false, refusal: 'spawn-failed' };
      if (typeof res.epoch === 'number') resetEpoch = res.epoch;
      clearLocalStorageAndReload();
      return { ok: true };
    },
    resolveSessionCwd: (sessionId) => (
      sessionId
        ? req(`/api/v1/usage/resolve-session?sessionId=${encodeURIComponent(sessionId)}`).then((r) => r?.cwd ?? null).catch(() => null)
        : Promise.resolve(null)
    ),
    copyToClipboard: async (text) => {
      const value = String(text ?? '');
      try {
        if (navigator.clipboard?.writeText) {
          await navigator.clipboard.writeText(value);
          return { ok: true };
        }
      } catch {}
      try {
        const ta = document.createElement('textarea');
        ta.value = value;
        ta.style.position = 'fixed';
        ta.style.opacity = '0';
        document.body.appendChild(ta);
        ta.select();
        const copied = document.execCommand('copy');
        ta.remove();
        return copied ? { ok: true } : { ok: false, error: 'copy failed' };
      } catch (e) {
        return { ok: false, error: String(e) };
      }
    },
    readClipboard: async () => {
      try {
        return navigator.clipboard?.readText ? await navigator.clipboard.readText() : '';
      } catch {
        return '';
      }
    },
    openExternal: async (url) => {
      if (typeof url === 'string' && /^https?:\/\//i.test(url)) {
        window.open(url, '_blank', 'noopener,noreferrer');
        return { ok: true };
      }
      return { ok: false, error: 'unsupported url' };
    },
    revealPath: (p) => {
      void baseApi.copyToClipboard(String(p ?? ''));
      return Promise.resolve({ ok: true, copied: true, path: p });
    },
    revealDamagedFile: (p) => {
      void baseApi.copyToClipboard(String(p ?? ''));
      return Promise.resolve({ ok: true, copied: true, path: p });
    },
    openTerminalAt: () => Promise.resolve({ ok: false, error: 'External terminal launch is desktop-only' }),
    openHtmlInBrowser: async (path) => {
      if (!path) return { ok: false, error: 'missing path' };
      window.open(`/api/v1/fs/raw?path=${encodeURIComponent(path)}`, '_blank', 'noopener,noreferrer');
      return { ok: true };
    },
    localOpenProjectFile: () => Promise.resolve({ ok: false, code: 'desktop_only', error: 'Desktop-only action' }),
    githubIssues: (repo) => req(`/api/v1/github/issues${repo ? `?repo=${encodeURIComponent(repo)}` : ''}`).then((r) => r || { ok: false, issues: [], error: 'request failed' }),

    // Usage, Memory, KG, Catalog
    agentUsage: (cwd, sessionId) => req(`/api/v1/usage/agent?cwd=${encodeURIComponent(cwd)}${sessionId ? `&sessionId=${encodeURIComponent(sessionId)}` : ''}`),
    agentContext: (agentId) => req(`/api/v1/hive/context/${encodeURIComponent(agentId)}`).then((r) => r?.context?.tokens ?? null),
    agentContextLimit: (agentId) => req(`/api/v1/hive/context/${encodeURIComponent(agentId)}`).then((r) => r?.context?.limit ?? null),
    setTokenLimit: (agentId, limit) => req('/api/v1/config/token-limit', { method: 'POST', body: JSON.stringify({ agentId, limit }) }),
    memoryStatus: () => req('/api/v1/memory/status'),
    hiveMemory: (agentId) => req(`/api/v1/memory/${encodeURIComponent(agentId)}`).then((r) => r?.content ?? ''),
    searchMemory: (q, limit) => req(`/api/v1/memory/search?q=${encodeURIComponent(q)}${limit ? `&limit=${limit}` : ''}`).then((r) => {
      if (r && typeof r.ok === 'boolean') return r;
      if (!Array.isArray(r)) return { ok: false, output: '', error: 'memory search failed' };
      const output = r.map((h) => `[${h.agentId || 'memory'}:${h.line ?? 0}] ${h.snippet || ''}`).join('\n');
      return { ok: true, output, hits: r };
    }),
    hiveAnswerQuestion: async (taskId, ref, change) => {
      const ledger = await baseApi.hiveTasks();
      const tasks = Array.isArray(ledger?.tasks) ? ledger.tasks : [];
      const task = tasks.find((t) => t.id === taskId || t.alias === taskId);
      if (!task) return { ok: false, error: 'gone' };
      const qa = Array.isArray(task.humanQA) ? task.humanQA.map((e) => ({ ...e })) : [];
      const idx = qa.findIndex((e) => e && e.q === ref?.q && !e.a && !e.dismissedAt && (!ref?.askedAt || e.askedAt === ref.askedAt));
      if (idx < 0) return { ok: false, error: 'gone' };
      qa[idx] = { ...qa[idx], ...change };
      return baseApi.hivePatchTask(task.id, { humanQA: qa });
    },
    hiveAnswerQuestions: (items) => Promise.all((Array.isArray(items) ? items : []).map((it) => baseApi.hiveAnswerQuestion(it.taskId, it.ref, it.change).then((r) => ({ taskId: it.taskId, ...r })))),
    kgStatus: () => req('/api/v1/kg/status'),
    kgList: () => req('/api/v1/kg/list'),
    kgSearch: (q, limit) => req(`/api/v1/kg/search?q=${encodeURIComponent(q)}${limit ? `&limit=${limit}` : ''}`),
    kgGet: (id) => req(`/api/v1/kg/docs/${encodeURIComponent(id)}`),
    kgRemove: (id) => req(`/api/v1/kg/docs/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    agentTemplates: () => req('/api/v1/templates/pro').then((r) => (Array.isArray(r) ? r : [])),
    projectsList: () => req('/api/v1/projects/list').then((r) => (r && Array.isArray(r.projects) ? r : { projects: [] })),
    onProjectsChanged: (fn) => addListener('projects:changed', fn),
    agentTemplateRead: (id) => req(`/api/v1/templates/pro/${encodeURIComponent(id)}`).then((r) => r || { ok: false, error: 'not found' }),
    agentFilePlan: (arg1, arg2) => {
      const payload = (arg1 && typeof arg1 === 'object' && 'file' in arg1 && arg2 === undefined)
        ? arg1
        : { agentId: arg1 || null, file: arg2 };
      return req('/api/v1/agent-file/plan', { method: 'POST', body: JSON.stringify(payload) }).then((r) => r || { ok: false, error: 'plan failed' });
    },
    agentFileSave: async (textOrObj, suggestedName) => {
      try {
        const text = typeof textOrObj === 'string' ? textOrObj : JSON.stringify(textOrObj, null, 2);
        const name = suggestedName || 'agent.agent.json';
        triggerDownload(name, text);
        return { ok: true, fileName: name };
      } catch (e) {
        return { ok: false, error: String(e) };
      }
    },
    agentFileOpen: async () => {
      const picked = await pickLocalFiles({ accept: '.json,.agent.json,application/json', multiple: false, readAs: 'text' });
      if (!picked || !picked.length) return { ok: false, error: 'cancelled' };
      try {
        const f = picked[0];
        const file = JSON.parse(f.text);
        return { ok: true, file, fileName: f.name };
      } catch (e) {
        return { ok: false, error: `Invalid JSON: ${e?.message || e}` };
      }
    },
    agentFileExport: (payload) => req('/api/v1/agent-file/export', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, error: 'export failed' }),
    agentFileApply: (payload) => req('/api/v1/agent-file/apply', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, error: 'apply failed' }),
    agentFileSkillChoices: (agentId) => req(`/api/v1/agent-file/skill-choices/${encodeURIComponent(agentId)}`).then((r) => (Array.isArray(r) ? r : [])),
    agentTemplateSave: (payload) => req('/api/v1/templates/pro/save', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, error: 'save failed' }),
    agentTemplateRemove: (id) => req(`/api/v1/templates/pro/${encodeURIComponent(id)}`, { method: 'DELETE' }).then((r) => r || { ok: false, error: 'remove failed' }),
    kgAddFiles: async () => {
      const picked = await pickLocalFiles({ accept: '.txt,.md,.markdown,.json,.csv,.tsv,.log,.html,.pdf', multiple: true, readAs: 'base64' });
      if (!picked || !picked.length) return { ok: false, error: 'cancelled', cancelled: true, results: [], added: 0 };
      let added = 0;
      const results = [];
      const docs = [];
      const errors = [];
      for (const f of picked) {
        const res = await req('/api/v1/kg/upload', {
          method: 'POST',
          body: JSON.stringify({ fileName: f.name, textBase64: f.base64, title: f.name }),
        }).catch(() => null);
        if (res?.ok) {
          added++;
          results.push({ ok: true, fileName: f.name, docId: res.docId, meta: res.meta });
          docs.push(res.meta);
        } else {
          const err = res?.error || f.name;
          results.push({ ok: false, fileName: f.name, error: err });
          errors.push(err);
        }
      }
      return { ok: added > 0, results, added, docs, errors };
    },
    saveDroppedFile: (name, bytes) => uploadRawBytes(name || 'drop.bin', bytes, 'drops'),
    saveClipboardImage: async () => {
      try {
        if (!navigator.clipboard?.read) return { ok: false, error: 'Clipboard image read unavailable' };
        const items = await navigator.clipboard.read();
        for (const item of items) {
          const imgType = item.types.find((t) => t.startsWith('image/'));
          if (imgType) {
            const blob = await item.getType(imgType);
            const buf = new Uint8Array(await blob.arrayBuffer());
            const ext = imgType.split('/')[1]?.split('+')[0] || 'png';
            return uploadRawBytes(`clipboard-${Date.now()}.${ext}`, buf, 'clipboard');
          }
        }
        return { ok: false, error: 'No image in clipboard' };
      } catch (e) {
        return { ok: false, error: String(e?.message || e) };
      }
    },
    attachFiles: async () => {
      const picked = await pickLocalFiles({ accept: '*/*', multiple: true, readAs: 'base64' });
      if (!picked || !picked.length) return { ok: false, files: [] };
      const files = [];
      for (const f of picked) {
        const res = await req('/api/v1/fs/upload', {
          method: 'POST',
          body: JSON.stringify({ name: f.name, base64: f.base64, scope: 'attachments' }),
        }).catch(() => null);
        if (res?.ok && res.path) files.push(res.path);
      }
      return { ok: files.length > 0, files };
    },
    attachToOffice: async () => baseApi.attachFiles(),
    importHireFiles: async () => baseApi.attachFiles(),
    hireRunnerReady: () => req('/api/v1/hive/hire-runner-ready', { method: 'POST' }).then((r) => r || { ok: false }),
    onHireRequest: (fn) => addListener('hire:request', fn),
    hireRequestDone: (reqId, answer) => req('/api/v1/hive/hire-done', { method: 'POST', body: JSON.stringify({ reqId, answer }) }).then((r) => r || { ok: false }),
    changeHome: async (nextPath) => {
      if (!nextPath) return { ok: false, error: 'Path required' };
      const res = await req('/api/v1/floor/switch', { method: 'POST', body: JSON.stringify({ harnessHome: nextPath }) });
      if (!res?.ok) return { ok: false, error: res?.detail || res?.refusal || 'Failed to switch folder' };
      if (typeof res.epoch === 'number') resetEpoch = res.epoch;
      clearLocalStorageAndReload();
      return { ok: true };
    },

    resetAll: async () => {
      try {
        resettingLocally = true;
        const r = await req('/api/v1/reset', { method: 'POST' });
        if (typeof r?.epoch === 'number') resetEpoch = r.epoch;
        clearLocalStorageAndReload();
        return r ?? { ok: true };
      } catch (e) {
        resettingLocally = false;
        return { ok: false, error: String(e) };
      }
    },

    backupStatus: () => req('/api/v1/backups/status'),
    backupSetDir: (path) => req('/api/v1/backups/set-dir', { method: 'POST', body: JSON.stringify({ path: path ?? null }) }),
    backupCreate: (stopFirst = false) => req('/api/v1/backups/create', { method: 'POST', body: JSON.stringify({ stopFirst: !!stopFirst }) }),
    backupImportFile: async (file) => {
      const buf = new Uint8Array(await file.arrayBuffer());
      const dataBase64 = arrayBufferToBase64(buf);
      return req('/api/v1/backups/import', { method: 'POST', body: JSON.stringify({ name: file.name, dataBase64 }) });
    },
    backupInspect: (name, targetHome) => req('/api/v1/backups/inspect', {
      method: 'POST',
      body: JSON.stringify({ name, ...(targetHome ? { targetHome } : {}) }),
    }),
    backupRestore: (name, cleanDirty = false, targetHome) => req('/api/v1/backups/restore', {
      method: 'POST',
      body: JSON.stringify({ name, cleanDirty: !!cleanDirty, ...(targetHome ? { targetHome } : {}) }),
    }).then((r) => {
      if (r?.ok) {
        if (typeof r.epoch === 'number') resetEpoch = r.epoch;
        clearLocalStorageAndReload();
      }
      return r || { ok: false, error: 'restore failed' };
    }),
    backupDelete: (name) => req(`/api/v1/backups/${encodeURIComponent(name)}`, { method: 'DELETE' }),
    backupDownloadUrl: (name) => `/api/v1/backups/${encodeURIComponent(name)}/download`,

    // Silenced external/omitted services & safe empty containers
    cloudList: () => Promise.resolve([]),
    officesList: () => Promise.resolve([]),
    cloudSelectedOffice: () => Promise.resolve(null),
    teamsMode: () => Promise.resolve('solo'),
    teamsMembership: () => Promise.resolve(null),
    teamsOrg: () => Promise.resolve(null),
    teamsPolicy: () => Promise.resolve(null),
    teamsConnection: () => Promise.resolve({ state: 'disconnected', reason: 'offline' }),
    teamsRoster: () => Promise.resolve({ ok: false, error: 'offline' }),
    hiveTakeDegraded: () => Promise.resolve([]),
    freeAccount: () => Promise.resolve(null),
    connectionsList: () => Promise.resolve([]),
    connectionsGrants: () => Promise.resolve([]),
    connectionsAllAgents: () => Promise.resolve({}),
    soloLicense: () => Promise.resolve({
      key: 'local-pro',
      state: 'active',
      renewsAt: null,
      deviceLabel: 'local-web',
      checkedAt: new Date().toISOString(),
      plan: 'pro',
    }),
    proPricing: () => Promise.resolve(null),
    billingSummary: () => Promise.resolve(null),
    updateCurrent: () => Promise.resolve({ state: 'disabled' }),
    slackStatus: () => Promise.resolve({ running: false, disabled: true }),
    slackHistory: () => Promise.resolve([]),
    slackAnswersIn: () => Promise.resolve([]),
    slackAnswerIn: () => Promise.resolve([]),
    webhooksStatus: () => req('/api/v1/webhooks/status').then((r) => r || { running: false, endpoints: [], port: null }),
    webhookStatus: () => req('/api/v1/webhooks/status').then((r) => r || { running: false, endpoints: [], port: null }),
    webhooksRetry: () => req('/api/v1/webhooks/retry', { method: 'POST', body: '{}' }).then((r) => r || { ok: false }),
    generateWebhookSecret: () => req('/api/v1/webhooks/generate-secret', { method: 'POST', body: '{}' }).then((r) => r?.secret || null),
    puckState: () => Promise.resolve({ shown: false, recording: null, recipient: null, recipients: [], busy: false, error: null }),
    puckConfig: () => Promise.resolve({ enabled: false }),
    puckMeetings: () => Promise.resolve([]),
    puckScreenshots: () => Promise.resolve([]),
    puckHistory: () => Promise.resolve([]),
    telemetrySnapshot: () => req('/api/v1/telemetry/snapshot').catch(() => null),
    telemetryUsage: (agentId) => req(`/api/v1/telemetry/usage${agentId ? `?agentId=${encodeURIComponent(agentId)}` : ''}`).catch(() => null),
    telemetrySpans: () => req('/api/v1/telemetry/spans').then((r) => (Array.isArray(r) ? r : [])).catch(() => []),
    trackMessageSent: () => Promise.resolve(),
    trackFunnel: () => Promise.resolve(),
    usageLimits: () => req('/api/v1/usage/limits').then((r) => {
      const engines = Array.isArray(r?.engines) ? r.engines : [];
      return {
        engines: engines.map((e) => ({
          ...e,
          engine: e?.engine || e?.id || 'claude',
          state: e?.state || 'not-available',
          plan: e?.plan ?? null,
          failed: !!e?.failed,
          asOf: e?.asOf ?? null,
          source: e?.source || 'local',
          windows: Array.isArray(e?.windows) ? e.windows : [],
        })),
      };
    }),
    tokenLimitsView: (agentIds) => req('/api/v1/token-limits/view', { method: 'POST', body: JSON.stringify({ agentIds: Array.isArray(agentIds) ? agentIds : [] }) }).then((r) => r || { office: { limit: null, period: 'day', used: 0, resetsAt: Date.now() }, agents: {} }),
    listWorkers: () => req('/api/v1/workers').then((r) => (r && Array.isArray(r.live) ? r : { live: [], preserved: [], maxWorkers: 4 })),
    workersHistory: () => req('/api/v1/workers/history').then((r) => (
      Array.isArray(r) ? r.map((row, idx) => ({
        id: row.id || `${row.workerId || 'worker'}-${row.endedAt || idx}`,
        workerId: row.workerId || 'worker',
        reqId: row.reqId || row.workerId || '',
        name: row.name || row.workerId || 'Temp',
        job: typeof row.job === 'string' ? row.job : (row.objective || ''),
        firstPrompt: typeof row.firstPrompt === 'string' ? row.firstPrompt : (row.job || ''),
        finalResponse: row.finalResponse || null,
        baseBranch: row.baseBranch || 'main',
        spawnedAt: row.spawnedAt ?? row.endedAt ?? Date.now(),
        endedAt: row.endedAt ?? Date.now(),
        tokensUsed: row.tokensUsed ?? 0,
        tokenCap: row.tokenCap ?? null,
        result: row.result || 'stopped',
        worktree: row.worktree ?? null,
        hasSlack: !!row.hasSlack,
        model: row.model ?? null,
        effort: row.effort ?? null,
        ...row,
      })) : []
    )),
    stopWorker: (workerId) => req(`/api/v1/workers/${encodeURIComponent(workerId)}/stop`, { method: 'POST', body: '{}' }).then((r) => r || { ok: false }),
    listMissions: () => req('/api/v1/missions').then((r) => (Array.isArray(r) ? r : [])),
    saveMissions: (missions) => req('/api/v1/missions', { method: 'POST', body: JSON.stringify(Array.isArray(missions) ? missions : []) }).then((r) => r || { ok: false }),
    listWebhooks: () => req('/api/v1/webhooks/endpoints').then((r) => (Array.isArray(r) ? r : [])),
    saveWebhooks: (list) => req('/api/v1/webhooks/save', { method: 'POST', body: JSON.stringify(Array.isArray(list) ? list : []) }).then((r) => (Array.isArray(r) ? r : null)),
    deleteWebhook: (id) => req(`/api/v1/webhooks/endpoints/${encodeURIComponent(id)}`, { method: 'DELETE' }).then(() => req('/api/v1/webhooks/endpoints')),
    telegramSetWebhook: (webhookUrl, token) => req('/api/v1/webhooks/telegram-set', { method: 'POST', body: JSON.stringify({ webhookUrl, token }) }).then((r) => r || { ok: false }),
    listTriggerHistory: () => req('/api/v1/triggers/history').then((r) => (Array.isArray(r) ? r : [])),
    decideTriggerHistory: ({ id, decision, taskId }) => req(`/api/v1/triggers/history/${encodeURIComponent(id)}/decide`, { method: 'POST', body: JSON.stringify({ decision, taskId }) }).then((r) => r?.entry ?? null),
    providerKeyHas: (backend) => req('/api/v1/secrets').then((list) => Array.isArray(list) && list.some((s) => s.keyRef === `apikey:${backend}`)),
    providerKeySet: ({ backend, key }) => req('/api/v1/secrets', { method: 'POST', body: JSON.stringify({ keyRef: `apikey:${backend}`, kind: 'provider', value: key }) }).then((r) => r || { ok: false }),
    providerKeyClear: (backend) => req(`/api/v1/secrets/${encodeURIComponent(`apikey:${backend}`)}`, { method: 'DELETE' }).then((r) => r || { ok: false }),
    customSecretList: () => req('/api/v1/secrets').then((list) => (Array.isArray(list) ? list.filter((s) => s.kind === 'custom').map((s) => s.keyRef) : [])),
    customSecretSet: ({ name, value }) => req('/api/v1/secrets', { method: 'POST', body: JSON.stringify({ keyRef: name, kind: 'custom', value }) }).then((r) => r || { ok: false }),
    customSecretRemove: (name) => req(`/api/v1/secrets/${encodeURIComponent(name)}`, { method: 'DELETE' }).then((r) => r || { ok: false }),
    customSecretApply: (payload) => req('/api/v1/secrets/apply', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, code: 'office_unreachable' }),
    integrationsList: () => req('/api/v1/integrations').then((r) => (Array.isArray(r) ? r : [])),
    integrationsSave: (payload) => req('/api/v1/integrations/save', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, error: 'request failed' }),
    integrationsRemove: (arg) => {
      const id = typeof arg === 'string' ? arg : arg?.id;
      return id ? req(`/api/v1/integrations/${encodeURIComponent(id)}`, { method: 'DELETE' }).then((r) => r || { ok: false }) : Promise.resolve({ ok: false });
    },
    integrationsTest: (payload) => req('/api/v1/integrations/test', { method: 'POST', body: JSON.stringify(payload || {}) }).then((r) => r || { ok: false, error: 'request failed' }),
    integrationsActivity: () => req('/api/v1/integrations/activity').then((r) => (Array.isArray(r) ? r : [])),
    integrationsKeyCheckAnswer: () => Promise.resolve({ ok: true }),
    officeThemesList: () => req('/api/v1/themes/office').then((r) => r || { themes: [], bad: [] }),
    officeThemesRead: (id) => req(`/api/v1/themes/office/${encodeURIComponent(id)}`),
    codeThemeList: () => req('/api/v1/themes/code').then((r) => (Array.isArray(r) ? r : [])),
    capabilitiesCatalog: () => req('/api/v1/catalog/capabilities').then((r) => r || { version: 1, entries: [], purposes: [] }),
    capabilitiesCatalogLook: () => req('/api/v1/catalog/capabilities').then((r) => r || { version: 1, entries: [], purposes: [] }),
    capabilitiesInstalled: (body) => req('/api/v1/catalog/capabilities/installed', { method: 'POST', body: JSON.stringify(body || {}) }).then((r) => r || { plugins: [], servers: [], tools: [] }),
    capabilitiesInstall: (entryId, _officeId) => req('/api/v1/capabilities/install', { method: 'POST', body: JSON.stringify({ entryId }) }).then((r) => r || { ok: false, lines: [] }),
    onCapabilitiesInstallLine: (fn) => addListener('capabilities:installLine', fn),
    capabilitiesUnavailable: () => Promise.resolve([]),
    setAgentMcp: (agentId, serverId, value) => req('/api/v1/agents/settings', { method: 'POST', body: JSON.stringify({ agentId, kind: 'mcp', key: serverId, value }) }),
    setAgentSkillOff: (agentId, folder, off) => req('/api/v1/agents/settings', { method: 'POST', body: JSON.stringify({ agentId, kind: 'skillOff', key: folder, value: !!off }) }),
    skillsOffAtLaunch: () => req('/api/v1/config').then((c) => c?.agentSkillsOff || {}).catch(() => ({})),
    setAgentCodexMemory: (agentId, on) => req('/api/v1/agents/settings', { method: 'POST', body: JSON.stringify({ agentId, kind: 'codexMemory', value: !!on }) }),
    setAgentBrowser: (agentId, value) => req('/api/v1/agents/settings', { method: 'POST', body: JSON.stringify({ agentId, kind: 'browser', value }) }),
    setAgentConnection: () => Promise.resolve({ ok: true }),
    skillsCatalog: () => req('/api/v1/skills').then((r) => {
      if (r && Array.isArray(r.skills)) return r;
      const list = Array.isArray(r) ? r : [];
      const skills = list.map((s) => (typeof s === 'string'
        ? { name: s, description: '', owner: 'bundled', category: 'general', url: `skill://${s}` }
        : s));
      return { skills, fetchedAt: Date.now(), stale: false };
    }),
    skillsLocal: () => req('/api/v1/skills').then((r) => {
      const list = Array.isArray(r) ? r : [];
      return list.map((s) => (typeof s === 'string'
        ? { name: s, description: '', provider: 'claude', scope: 'bundled', path: `skills/${s}` }
        : s));
    }),
    skillsInstall: (url, name) => req('/api/v1/skills/install', { method: 'POST', body: JSON.stringify({ url, name }) }).then((r) => r || { ok: false }),
    skillsUninstall: (path) => req('/api/v1/skills/uninstall', { method: 'POST', body: JSON.stringify({ path }) }).then((r) => r || { ok: false }),
    skillsReveal: (path) => {
      void baseApi.copyToClipboard(String(path ?? ''));
      return Promise.resolve({ ok: true, copied: true, path });
    },
    fileShareList: () => req('/api/v1/share').then((r) => (Array.isArray(r) ? r : [])),
    fileShareRevoke: (id) => req(`/api/v1/share/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    fileShareStopAll: () => req('/api/v1/share/stop-all', { method: 'POST', body: '{}' }).then((r) => r || { ok: false }),
    fileShareInspect: (paths) => req('/api/v1/share/inspect', { method: 'POST', body: JSON.stringify({ paths: Array.isArray(paths) ? paths : [] }) }).then((r) => (Array.isArray(r) ? r : [])),
    fileShareStatus: () => req('/api/v1/share/status', { method: 'POST', body: '{}' }).then((r) => (Array.isArray(r) ? r : [])),
    wakeAgent: (agentId) => req('/api/v1/hive/wake', { method: 'POST', body: JSON.stringify({ agentId }) }).then((r) => r || { ok: false }),
    hiveWakeRetry: (agentId) => req('/api/v1/hive/wake-retry', { method: 'POST', body: JSON.stringify({ agentId }) }).then((r) => r || { ok: false }),
    hiveWakeText: (agentId) => req(`/api/v1/hive/wake-text/${encodeURIComponent(agentId)}`),
    hiveWakeUnconfirmed: (agentId) => req(`/api/v1/hive/wake-unconfirmed/${encodeURIComponent(agentId)}`).then((r) => (Array.isArray(r) ? r : [])),
    hiveSwitchClearGate: (id) => req(`/api/v1/hive/switch-clear-gate/${encodeURIComponent(id)}`).catch(() => 'drop'),
    hiveCodexSizeClearText: (id) => req(`/api/v1/hive/codex-size-clear-text/${encodeURIComponent(id)}`).catch(() => null),
    hiveEndedAgents: () => req('/api/v1/hive/ended-agents').then((r) => (Array.isArray(r) ? r : [])).catch(() => []),
    voiceTerminal: (ptyId, _rows, lines = 40) => req('/api/v1/pty/screen-lines', { method: 'POST', body: JSON.stringify({ ptyId, lines }) }).then((r) => r || { ok: false, lines: [] }),
    ideAttach: (handoff) => {
      try {
        window.localStorage.setItem('cth.ide.attached', JSON.stringify({ attached: true, handoff: handoff || null, at: Date.now() }));
        ideChannel?.postMessage({ type: 'attach', handoff: handoff || null });
      } catch {}
      return Promise.resolve({ ok: true, attached: true });
    },
    ideDetach: (handoff) => {
      try {
        window.localStorage.setItem('cth.ide.attached', JSON.stringify({ attached: false, handoff: handoff || null, at: Date.now() }));
        ideChannel?.postMessage({ type: 'detach', handoff: handoff || null });
        window.open('/ide', 'cth-ide');
      } catch {}
      return Promise.resolve({ ok: true, attached: false });
    },
    ideWindowState: () => {
      try {
        const raw = window.localStorage.getItem('cth.ide.attached');
        if (raw) return Promise.resolve(JSON.parse(raw));
      } catch {}
      return Promise.resolve({ attached: false, open: false });
    },
    ideRecall: () => Promise.resolve({ ok: true }),
    ideAttachedAck: () => Promise.resolve({ ok: true }),
    ideAgents: () => req('/api/v1/roster').then((r) => (Array.isArray(r?.agents) ? r.agents : [])),
    heroPayload: () => Promise.resolve({ hero: null }),
    getWindowVisibility: () => Promise.resolve(!document.hidden),
    tourBoot: () => Promise.resolve({ seen: true, step: null }),
    tourSeen: () => {
      try { window.localStorage.setItem('cth.tour.seen', '1'); } catch {}
      return Promise.resolve({ ok: true });
    },
    devNoGod: () => Promise.resolve(false),
    graphicsStatus: () => Promise.resolve({ mode: 'browser', hardwareAcceleration: true, available: true }),
    graphicsRestart: () => Promise.resolve({ ok: true }),
    setGraphicsMode: (mode) => Promise.resolve({ ok: true, mode }),
    cancelClose: () => Promise.resolve({ ok: true }),
    confirmClose: () => Promise.resolve({ ok: true }),
    installCli: () => Promise.resolve({ ok: false, error: 'CLI installer is only available in the desktop app or via terminal' }),
    hiveArchivedAtBoot: () => Promise.resolve([]),
    hiveHeldAtBoot: () => Promise.resolve([]),
    drainPendingHires: () => Promise.resolve([]),
    transcribeStatus: () => req('/api/v1/config').then((c) => buildTranscribeStatus(c?.transcribe)).catch(() => buildTranscribeStatus(null)),
    transcribeSetConfig: async (patch) => {
      const cur = await req('/api/v1/config').catch(() => null);
      const nextTranscribe = { ...(cur?.transcribe || {}), ...(patch || {}) };
      const updated = await baseApi.updateConfig({ transcribe: nextTranscribe });
      return { ok: !!updated, status: buildTranscribeStatus(updated?.transcribe || nextTranscribe) };
    },
    freeflowSetConfig: () => Promise.resolve({ ok: true }),
    realtimeHasOpenAiKey: () => req('/api/v1/realtime/has-key').then((r) => !!r?.hasKey),
    realtimeMintToken: (p) => req('/api/v1/realtime/mint-token', { method: 'POST', body: JSON.stringify(p || {}) }),
    realtimeAction: () => Promise.resolve({ ok: false, error: 'Realtime actions are disabled in web mode' }),
    realtimeActionConfirm: () => Promise.resolve({ ok: false, error: 'Realtime actions are disabled in web mode' }),
    realtimeDrainCompletions: () => Promise.resolve([]),
    realtimeSetSessionLive: () => Promise.resolve({ ok: true }),
    realtimeWaitFor: () => Promise.resolve({ ok: false, timedOut: true }),
  };

  window.cth = new Proxy(baseApi, {
    get(target, prop) {
      if (prop in target) return target[prop];
      const name = String(prop);
      if (name.startsWith('on') && name.length > 2 && name[2] === name[2].toUpperCase()) {
        const ch = name.slice(2, 3).toLowerCase() + name.slice(3);
        return (fn) => addListener(ch, fn);
      }
      return async () => null;
    },
  });
})();
