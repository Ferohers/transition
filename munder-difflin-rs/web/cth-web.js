(() => {
  const listeners = new Map();
  const ptySockets = new Map();

  function addListener(channel, fn) {
    if (typeof fn !== 'function') return () => {};
    if (!listeners.has(channel)) listeners.set(channel, new Set());
    listeners.get(channel).add(fn);
    return () => listeners.get(channel)?.delete(fn);
  }

  function emitLocal(channel, payload) {
    const set = listeners.get(channel);
    if (!set) return;
    for (const fn of set) {
      try { fn(payload); } catch (e) { console.error('[cth-web listener]', channel, e); }
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

  function connectEventsWs() {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${proto}//${location.host}/ws/events`);
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg && msg.channel) emitLocal(msg.channel, msg.payload);
      } catch {}
    };
    ws.onclose = () => setTimeout(connectEventsWs, 1500);
  }
  connectEventsWs();

  function ensurePtySocket(id) {
    if (!id || ptySockets.has(id)) return;
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${proto}//${location.host}/ws/pty/${encodeURIComponent(id)}`);
    ws.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg.type === 'data') emitLocal(`pty:data:${id}`, msg.data);
        else if (msg.type === 'exit') emitLocal(`pty:exit:${id}`, msg);
      } catch {
        emitLocal(`pty:data:${id}`, ev.data);
      }
    };
    ws.onclose = () => ptySockets.delete(id);
    ptySockets.set(id, ws);
  }

  const baseApi = {
    platform: 'darwin',
    arch: 'arm64',
    version: '0.5.10',
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
    updateConfig: (patch) => req('/api/v1/config', { method: 'PATCH', body: JSON.stringify(patch || {}) }),
    ensureHarnessHome: (path) => req('/api/v1/config/ensure-home', { method: 'POST', body: JSON.stringify({ path }) }),
    integrityIssues: () => req('/api/v1/integrity/issues'),
    rosterReadSync: () => syncGetJson('/api/v1/roster', null),
    rosterWrite: (snap) => req('/api/v1/roster', { method: 'PUT', body: JSON.stringify(snap) }),
    historyAdd: (entry) => req('/api/v1/history', { method: 'POST', body: JSON.stringify(entry) }),
    historyList: (agentId, limit) => req(`/api/v1/history?${new URLSearchParams({ ...(agentId ? { agentId } : {}), ...(limit ? { limit: String(limit) } : {}) })}`),
    historySearch: (q, limit) => req(`/api/v1/history?${new URLSearchParams({ q, ...(limit ? { limit: String(limit) } : {}) })}`),

    // PTY
    spawnPty: async (opts) => {
      const res = await req('/api/v1/pty/spawn', { method: 'POST', body: JSON.stringify(opts) });
      if (res?.ok && res.id) ensurePtySocket(res.id);
      return res;
    },
    writePty: (id, data) => req('/api/v1/pty/write', { method: 'POST', body: JSON.stringify({ id, data }) }),
    resizePty: (id, cols, rows) => req('/api/v1/pty/resize', { method: 'POST', body: JSON.stringify({ id, cols, rows }) }),
    redrawPty: (id) => req('/api/v1/pty/redraw', { method: 'POST', body: JSON.stringify({ id }) }),
    killPty: (id) => req('/api/v1/pty/kill', { method: 'POST', body: JSON.stringify({ id }) }),
    listPtys: () => req('/api/v1/pty/list'),
    submitToPty: (payload) => req('/api/v1/pty/submit', { method: 'POST', body: JSON.stringify(payload) }),
    withdrawPtySubmit: (ref) => req('/api/v1/pty/withdraw', { method: 'POST', body: JSON.stringify({ ref }) }),
    onPtyData: (id, fn) => { ensurePtySocket(id); return addListener(`pty:data:${id}`, fn); },
    onPtyExit: (id, fn) => addListener(`pty:exit:${id}`, fn),
    codexAllowedFolders: () => req('/api/v1/codex/allowed-folders'),
    codexAllowFolder: (folder) => req('/api/v1/codex/allow-folder', { method: 'POST', body: JSON.stringify({ folder }) }),
    codexWithdrawFolder: (folder) => req('/api/v1/codex/withdraw-folder', { method: 'POST', body: JSON.stringify({ folder }) }),

    // Hive & Controls
    hiveRegistry: () => req('/api/v1/hive/registry'),
    hiveTasks: () => req('/api/v1/hive/tasks'),
    hiveTasksArchive: () => req('/api/v1/hive/tasks-archive'),
    hiveAddTask: (task) => req('/api/v1/hive/tasks', { method: 'POST', body: JSON.stringify(task) }),
    hivePatchTask: (id, patch) => req(`/api/v1/hive/tasks/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify(patch) }),
    hiveDeleteTask: (id) => req(`/api/v1/hive/tasks/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    hiveBoard: () => req('/api/v1/hive/board'),
    hiveLog: (limit) => req(`/api/v1/hive/log${limit ? `?limit=${limit}` : ''}`),
    hiveInbox: (agentId) => req(`/api/v1/hive/inbox/${encodeURIComponent(agentId)}`),
    hiveMessages: (agentId) => req(`/api/v1/hive/messages/${encodeURIComponent(agentId)}`),
    textSearch: (q) => req(`/api/v1/hive/search?q=${encodeURIComponent(q)}`),
    controlSnapshot: () => req('/api/v1/control/snapshot'),
    controlPause: (agentId, reason) => req('/api/v1/control/pause', { method: 'POST', body: JSON.stringify({ agentId, reason }) }),
    controlResume: (agentId) => req('/api/v1/control/resume', { method: 'POST', body: JSON.stringify({ agentId }) }),
    controlSteer: (agentId, message) => req('/api/v1/control/steer', { method: 'POST', body: JSON.stringify({ agentId, message }) }),
    controlHalt: (reason) => req('/api/v1/control/halt', { method: 'POST', body: JSON.stringify({ reason }) }),
    controlUnhalt: () => req('/api/v1/control/unhalt', { method: 'POST', body: JSON.stringify({}) }),

    // FS & Git
    listDir: (root, rel = '') => req('/api/v1/fs/list', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    readFile: (root, rel) => req('/api/v1/fs/read', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    readBinary: (root, rel) => req('/api/v1/fs/read-binary', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    writeFile: (root, rel, content) => req('/api/v1/fs/write', { method: 'POST', body: JSON.stringify({ root, rel, content }) }),
    searchFiles: (root, query, opts) => req('/api/v1/fs/search', { method: 'POST', body: JSON.stringify({ root, query, opts }) }),
    createFile: (root, rel) => req('/api/v1/fs/create', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    renamePath: (root, from, to) => req('/api/v1/fs/rename', { method: 'POST', body: JSON.stringify({ root, from, to }) }),
    trashPath: (root, rel) => req('/api/v1/fs/trash', { method: 'POST', body: JSON.stringify({ root, rel }) }),
    statAbs: (path) => req('/api/v1/fs/stat-abs', { method: 'POST', body: JSON.stringify({ path }) }),
    chooseFolder: () => {
      const p = window.prompt('Workspace folder path:', '~/HarnessAgents');
      return Promise.resolve(p && p.trim() ? { ok: true, path: p.trim() } : { ok: false, error: 'cancelled' });
    },
    toolsStatus: () => req('/api/v1/tools/status'),
    hiveTheme: () => req('/api/v1/config').then((c) => c?.officeTheme ?? null),
    setHiveTheme: (_home, officeTheme) => req('/api/v1/config', { method: 'PATCH', body: JSON.stringify({ officeTheme }) }).then((c) => ({ ok: !!c })),
    teamsSetBossName: () => Promise.resolve({ ok: true, offline: true }),
    teamsSetName: () => Promise.resolve({ ok: true, offline: true }),
    hiveRenameAgent: () => Promise.resolve({ ok: true }),
    hivePatchAgentRole: () => Promise.resolve({ ok: true }),
    hivePatchAgentGoal: () => Promise.resolve({ ok: true }),
    hivePatchAgentLook: () => Promise.resolve({ ok: true }),
    hiveSetAgentHold: () => Promise.resolve({ ok: true }),
    hiveSetArchived: () => Promise.resolve({ ok: true }),
    reportFloorState: () => Promise.resolve({ ok: true }),
    cliMissingState: () => Promise.resolve(null),
    cliSetupState: () => Promise.resolve(null),
    codexHeldFolders: () => Promise.resolve(null),
    codexWaitingFolders: () => Promise.resolve([]),
    gitIsRepo: (cwd) => req(`/api/v1/git/is-repo?cwd=${encodeURIComponent(cwd)}`),
    gitBranch: (cwd) => req(`/api/v1/git/branch?cwd=${encodeURIComponent(cwd)}`),
    gitStatus: (cwd) => req(`/api/v1/git/status?cwd=${encodeURIComponent(cwd)}`),
    gitLog: (cwd, n) => req(`/api/v1/git/log?cwd=${encodeURIComponent(cwd)}${n ? `&n=${n}` : ''}`),
    gitDiff: (cwd, relPath) => req(`/api/v1/git/diff?cwd=${encodeURIComponent(cwd)}&relPath=${encodeURIComponent(relPath)}`),

    // Usage, Memory, KG, Catalog
    agentUsage: (cwd, sessionId) => req(`/api/v1/usage/agent?cwd=${encodeURIComponent(cwd)}${sessionId ? `&sessionId=${encodeURIComponent(sessionId)}` : ''}`),
    hiveMemory: (agentId) => req(`/api/v1/memory/${encodeURIComponent(agentId)}`).then((r) => r?.content ?? ''),
    searchMemory: (q, limit) => req(`/api/v1/memory/search?q=${encodeURIComponent(q)}${limit ? `&limit=${limit}` : ''}`),
    kgStatus: () => req('/api/v1/kg/status'),
    kgList: () => req('/api/v1/kg/list'),
    kgSearch: (q, limit) => req(`/api/v1/kg/search?q=${encodeURIComponent(q)}${limit ? `&limit=${limit}` : ''}`),
    kgGet: (id) => req(`/api/v1/kg/docs/${encodeURIComponent(id)}`),
    kgRemove: (id) => req(`/api/v1/kg/docs/${encodeURIComponent(id)}`, { method: 'DELETE' }),
    agentTemplates: () => req('/api/v1/templates'),

    // Silenced external/omitted services & safe empty containers
    cloudList: () => Promise.resolve([]),
    officesList: () => Promise.resolve([]),
    cloudSelectedOffice: () => Promise.resolve(null),
    teamsMode: () => Promise.resolve('solo'),
    teamsMembership: () => Promise.resolve(null),
    teamsOrg: () => Promise.resolve(null),
    teamsPolicy: () => Promise.resolve(null),
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
    telemetrySnapshot: () => Promise.resolve(null),
    telemetryUsage: () => Promise.resolve(null),
    telemetrySpans: () => Promise.resolve([]),
    trackMessageSent: () => Promise.resolve(),
    trackFunnel: () => Promise.resolve(),
    usageLimits: () => Promise.resolve({ engines: [] }),
    tokenLimitsView: () => Promise.resolve({}),
    listWorkers: () => Promise.resolve([]),
    workersHistory: () => Promise.resolve([]),
    listMissions: () => Promise.resolve([]),
    listWebhooks: () => Promise.resolve([]),
    listTriggerHistory: () => Promise.resolve([]),
    customSecretList: () => Promise.resolve([]),
    integrationsList: () => Promise.resolve([]),
    officeThemesList: () => Promise.resolve({ themes: [], bad: [] }),
    codeThemeList: () => Promise.resolve([]),
    capabilitiesCatalog: () => Promise.resolve([]),
    capabilitiesInstalled: () => Promise.resolve([]),
    skillsCatalog: () => Promise.resolve([]),
    skillsLocal: () => Promise.resolve([]),
    fileShareList: () => Promise.resolve([]),
    hiveArchivedAtBoot: () => Promise.resolve([]),
    hiveHeldAtBoot: () => Promise.resolve([]),
    hiveEndedAgents: () => Promise.resolve([]),
    drainPendingHires: () => Promise.resolve([]),
    transcribeStatus: () => Promise.resolve({ chosen: { dictation: null }, config: { dictation: false } }),
    realtimeHasOpenAiKey: () => req('/api/v1/realtime/has-key').then((r) => !!r?.hasKey),
    realtimeMintToken: (p) => req('/api/v1/realtime/mint-token', { method: 'POST', body: JSON.stringify(p || {}) }),
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
