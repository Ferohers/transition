const stage = document.getElementById('floor-stage');
const boxEl = document.getElementById('floor-box');
const floorDesks = document.getElementById('floor-desks');
const agentListEl = document.getElementById('agent-list');
const floorDimsEl = document.getElementById('floor-dims');
const wsBadgeEl = document.getElementById('ws-badge');
const termOutEl = document.getElementById('term-out');
const themeFrame = document.getElementById('theme-sandbox');

let activePtyId = null;
let ptySocket = null;
let pinnedSandboxOrigin = typeof Origin !== 'undefined' ? new Origin() : null;
let sandboxPinned = false;

async function api(path, opts = {}) {
  const res = await fetch(path, {
    headers: { 'content-type': 'application/json', ...(opts.headers || {}) },
    ...opts,
  });
  return res.json();
}

function switchTab(name) {
  document.querySelectorAll('.pro-nav-btn[data-tab]').forEach((b) => {
    b.classList.toggle('active', b.dataset.tab === name);
  });
  document.querySelectorAll('.view-pane').forEach((p) => {
    p.classList.toggle('active', p.id === `tab-${name}`);
  });
  if (name === 'floor') void layoutFloor();
}

document.querySelectorAll('.pro-nav-btn[data-tab]').forEach((btn) => {
  btn.addEventListener('click', () => switchTab(btn.dataset.tab));
});

// Patch 2: Enlarged Main Office Area (homeBoxOf floor viewport fill)
async function layoutFloor() {
  if (!stage || !boxEl) return;
  const r = stage.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return;
  const b = await api(`/api/v1/floor/box?home_w=${r.width}&stage_h=${r.height}`);
  boxEl.style.left = `${b.x}px`;
  boxEl.style.top = `${b.y}px`;
  boxEl.style.width = `${b.w}px`;
  boxEl.style.height = `${b.h}px`;
  floorDimsEl.textContent = `${Math.round(b.w)} × ${Math.round(b.h)} px (Patch 2 Floor Fill)`;
}

async function refreshAgents() {
  const reg = await api('/api/v1/hive/registry');
  const agents = Array.isArray(reg?.agents) ? reg.agents : [];
  agentListEl.innerHTML = agents.length
    ? agents.map((a) => `<div class="pro-row"><span>${a.name || a.id}</span><span class="pro-chip">${a.engine || 'claude'}</span></div>`).join('')
    : '<div class="pro-chip">No agents yet</div>';
  floorDesks.innerHTML = agents.length
    ? agents.map((a) => `<div class="desk-card"><strong>${a.name || a.id}</strong><div class="pro-chip">${a.role || 'agent'}</div></div>`).join('')
    : '<div class="desk-card"><strong>Main Floor Ready</strong><div class="pro-chip">Use + Spawn Shell or Add Agent</div></div>';
}

async function refreshTasks() {
  const tasks = await api('/api/v1/hive/tasks');
  const list = Array.isArray(tasks) ? tasks : [];
  document.getElementById('tasks-list').innerHTML = list.length
    ? list.map((t) => `<div class="desk-card"><strong>${t.id}: ${t.title}</strong> <span class="pro-chip">${t.status || 'todo'}</span></div>`).join('')
    : '<div class="pro-chip">No tasks yet</div>';
}

async function refreshCatalog() {
  const [templates, presets, skills] = await Promise.all([
    api('/api/v1/templates'),
    api('/api/v1/presets'),
    api('/api/v1/skills'),
  ]);
  document.getElementById('catalog-summary').textContent =
    `${templates.length} templates · ${presets.length} presets · ${skills.length} skills`;
}

document.getElementById('btn-spawn-shell').addEventListener('click', async () => {
  const res = await api('/api/v1/pty/spawn', {
    method: 'POST',
    body: JSON.stringify({ id: `shell-${Date.now()}`, cols: 100, rows: 28 }),
  });
  if (res.ok) {
    activePtyId = res.id;
    termOutEl.textContent = `[attached ${activePtyId}]\n`;
    if (ptySocket) ptySocket.close();
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
    ptySocket = new WebSocket(`${proto}//${location.host}/ws/pty/${encodeURIComponent(activePtyId)}`);
    ptySocket.onmessage = (ev) => {
      termOutEl.textContent += ev.data;
      termOutEl.scrollTop = termOutEl.scrollHeight;
    };
    switchTab('terminal');
  }
});

document.getElementById('term-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const inp = document.getElementById('term-input');
  if (!activePtyId || !inp.value) return;
  await api('/api/v1/pty/write', {
    method: 'POST',
    body: JSON.stringify({ id: activePtyId, data: `${inp.value}\n` }),
  });
  inp.value = '';
});

document.getElementById('theme-toggle').addEventListener('click', async () => {
  const root = document.documentElement;
  const next = root.dataset.cthTheme === 'dark' ? 'light' : 'dark';
  root.dataset.cthTheme = next;
  await api('/api/v1/themes/workbench', {
    method: 'POST',
    body: JSON.stringify({ theme: next }),
  });
  if (themeFrame?.contentWindow) {
    themeFrame.contentWindow.postMessage({ type: 'theme', theme: next }, '*');
  }
});

// Native <dialog> handling with returnValue reset
const addTaskDialog = document.getElementById('add-task-dialog');
document.getElementById('btn-open-task-dialog').addEventListener('click', () => addTaskDialog.showModal());
addTaskDialog.addEventListener('close', async () => {
  const action = addTaskDialog.returnValue;
  addTaskDialog.returnValue = '';
  if (action !== 'create') return;
  const title = document.getElementById('task-title-input').value.trim();
  if (!title) return;
  await api('/api/v1/hive/tasks', {
    method: 'POST',
    body: JSON.stringify({ id: `MD-${Date.now() % 10000}`, title }),
  });
  document.getElementById('task-title-input').value = '';
  await refreshTasks();
});

// Sandboxed iframe origin validation (modern-web-guidance: validate-origins)
window.addEventListener('message', (event) => {
  if (!themeFrame || event.source !== themeFrame.contentWindow) return;
  if (typeof Origin !== 'undefined') {
    try {
      const senderOrigin = Origin.from(event);
      if (!sandboxPinned && event.data?.type === 'sandbox-ready') {
        pinnedSandboxOrigin = senderOrigin;
        sandboxPinned = true;
      }
      if (!senderOrigin.isSameOrigin(pinnedSandboxOrigin)) return;
    } catch {
      return;
    }
  }
  if (event.data?.type === 'sandbox-ready') {
    document.getElementById('sandbox-status').textContent = 'Sandboxed theme iframe verified';
  }
});

function connectEventsWs() {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const ws = new WebSocket(`${proto}//${location.host}/ws/events`);
  ws.onopen = () => { wsBadgeEl.textContent = 'WS live'; };
  ws.onclose = () => {
    wsBadgeEl.textContent = 'WS reconnecting';
    setTimeout(connectEventsWs, 1500);
  };
  ws.onmessage = () => {
    void refreshAgents();
  };
}

new ResizeObserver(() => { void layoutFloor(); }).observe(stage);
void Promise.all([layoutFloor(), refreshAgents(), refreshTasks(), refreshCatalog()]);
connectEventsWs();
