// spectyn web frontend — single-page app for /
// served by `spectyn serve` from embedded core/web/ files.
'use strict';

const $ = (id) => document.getElementById(id);

// ─── Tab switching ───────────────────────────────────────────────
function activateTab(target) {
  document.querySelectorAll('.tab').forEach(b => {
    b.classList.toggle('active', b.dataset.tab === target);
  });
  document.querySelectorAll('.pane').forEach(p => {
    p.classList.toggle('active', p.id === `pane-${target}`);
  });
  if (target === 'info') refreshInfoActive();
}
document.querySelectorAll('.tab').forEach(btn => {
  btn.addEventListener('click', () => activateTab(btn.dataset.tab));
});

// ─── Sub-tabs (Info pane: Todo / Sessions / Cost) ────────────────
function activateSubtab(target) {
  document.querySelectorAll('.subtab').forEach(b => {
    b.classList.toggle('active', b.dataset.subtab === target);
  });
  document.querySelectorAll('.subpane').forEach(p => {
    p.classList.toggle('active', p.id === `subpane-${target}`);
  });
  refreshInfoActive();
}
document.querySelectorAll('.subtab').forEach(btn => {
  btn.addEventListener('click', () => activateSubtab(btn.dataset.subtab));
});
const infoRefreshBtn = document.getElementById('info-refresh');
if (infoRefreshBtn) infoRefreshBtn.addEventListener('click', refreshInfoActive);

// ─── Status / nodes (polled every 5s) ────────────────────────────
async function refreshStatus() {
  try {
    const res = await fetch('/api/status');
    if (!res.ok) throw new Error(`status ${res.status}`);
    const s = await res.json();
    if (s.version) $('version').textContent = `v${s.version}`;

    const pp = $('provider-pill');
    if (s.providers && s.providers.length) {
      pp.textContent = `● ${s.providers.length} provider${s.providers.length === 1 ? '' : 's'}`;
      pp.className = 'pill ok';
      pp.title = s.providers.join(', ');
    } else {
      pp.textContent = '⚠ no providers';
      pp.className = 'pill warn';
    }

    const cp = $('cluster-pill');
    if (s.cluster && s.cluster.peers > 0) {
      cp.textContent = `● ${s.cluster.peers} peer${s.cluster.peers === 1 ? '' : 's'}`;
      cp.className = 'pill ok';
    } else {
      cp.textContent = '○ standalone';
      cp.className = 'pill';
    }
  } catch (e) {
    $('provider-pill').textContent = '— offline';
    $('provider-pill').className = 'pill err';
  }
}

async function refreshNodes() {
  try {
    const res = await fetch('/api/nodes');
    if (!res.ok) throw new Error(`nodes ${res.status}`);
    const nodes = await res.json();
    const ul = $('node-list');
    if (!nodes || nodes.length === 0) {
      ul.innerHTML = '<li class="muted">standalone (no peers)</li>';
      return;
    }
    ul.innerHTML = nodes.map(n => {
      const cls = n.online ? 'online' : (n.status === 'unknown' ? 'unknown' : 'offline');
      const meta = n.online ? 'ok' : (n.status || 'offline');
      return `<li>
        <span class="node-dot ${cls}"></span>
        <span class="node-name">${escapeHtml(n.name || n.url || 'unknown')}</span>
        <span class="node-meta">${escapeHtml(meta)}</span>
      </li>`;
    }).join('');
  } catch (e) {
    $('node-list').innerHTML = '<li class="muted">— unable to load</li>';
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, c => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;'
  }[c]));
}

refreshStatus();
refreshNodes();
setInterval(refreshStatus, 5000);
setInterval(refreshNodes, 10000);

// ─── Terminal pane (xterm.js) ────────────────────────────────────
const termInput = $('prompt-input');
const sendBtn = $('send-btn');
const termHost = $('terminal-host');

const ANSI = {
  reset:  '\x1b[0m',
  dim:    '\x1b[2m',
  purple: '\x1b[35m',
  cyan:   '\x1b[36m',
  green:  '\x1b[32m',
  red:    '\x1b[31m',
  gray:   '\x1b[90m',
  bold:   '\x1b[1m',
};

const term = new Terminal({
  fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
  fontSize: 13,
  lineHeight: 1.4,
  cursorBlink: false,
  cursorStyle: 'block',
  disableStdin: true,                  // input goes through the bottom textarea
  convertEol: true,
  scrollback: 5000,
  theme: {
    background: '#1a1a1a',
    foreground: '#d4d4d4',
    cursor: '#c084fc',
    selectionBackground: '#3a3a3a',
    black:   '#1a1a1a', red:     '#fca5a5', green:   '#86efac', yellow:  '#fbbf24',
    blue:    '#7dd3fc', magenta: '#c084fc', cyan:    '#67e8f9', white:   '#d4d4d4',
    brightBlack:   '#666',     brightRed:     '#fca5a5', brightGreen:   '#86efac',
    brightYellow:  '#fbbf24',  brightBlue:    '#7dd3fc', brightMagenta: '#c084fc',
    brightCyan:    '#67e8f9',  brightWhite:   '#ffffff',
  },
});
const fit = new FitAddon.FitAddon();
term.loadAddon(fit);
term.open(termHost);
fit.fit();
window.addEventListener('resize', () => { try { fit.fit(); } catch(_) {} });
new ResizeObserver(() => { try { fit.fit(); } catch(_) {} }).observe(termHost);

// Welcome banner inside the xterm
term.writeln('');
term.writeln(`  ${ANSI.purple}spectyn${ANSI.reset} ${ANSI.gray}— web terminal${ANSI.reset}`);
term.writeln(`  ${ANSI.gray}type a prompt below and press Enter, or run \`spectyn\` in your shell${ANSI.reset}`);
term.writeln('');

function setBusy(busy) {
  sendBtn.disabled = busy;
  termInput.disabled = busy;
  sendBtn.textContent = busy ? '…' : 'send';
}

// auto-resize textarea
termInput.addEventListener('input', () => {
  termInput.style.height = 'auto';
  termInput.style.height = Math.min(200, termInput.scrollHeight) + 'px';
});

termInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    $('prompt-form').requestSubmit();
  }
});

$('prompt-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const prompt = termInput.value.trim();
  if (!prompt) return;
  termInput.value = '';
  termInput.style.height = 'auto';

  // Render user prompt (with ◆ glyph)
  const userLines = prompt.split('\n');
  term.writeln(`${ANSI.purple}◆${ANSI.reset} ${userLines[0]}`);
  for (const line of userLines.slice(1)) {
    term.writeln(`  ${ANSI.gray}·${ANSI.reset} ${line}`);
  }

  setBusy(true);
  let assistantStarted = false;
  let metaPrinted = false;

  try {
    const res = await fetch('/api/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ prompt }),
    });
    if (!res.ok) {
      const txt = await res.text();
      term.writeln(`${ANSI.red}error:${ANSI.reset} ${txt || 'request failed'}`);
      setBusy(false);
      return;
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      const lines = buf.split('\n');
      buf = lines.pop() || '';
      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const json = line.slice(6).trim();
        if (!json) continue;
        try {
          const ev = JSON.parse(json);
          if (ev.type === 'token') {
            assistantStarted = true;
            term.write((ev.content || '').replace(/\r?\n/g, '\r\n'));
          } else if (ev.type === 'thinking') {
            // Reasoning trace from extended-thinking / o1 / opencode reasoning
            // models. Render as muted italic so it's visually distinct from
            // the actual answer.
            const t = (ev.content || '').replace(/\r?\n/g, '\r\n');
            // ANSI dim+italic, gray, then reset.
            term.write(`${ANSI.gray}\x1b[2m\x1b[3m${t}\x1b[0m`);
          } else if (ev.type === 'tool_start') {
            if (assistantStarted) { term.write('\r\n'); assistantStarted = false; }
            term.writeln(`${ANSI.cyan}●${ANSI.reset} ${ANSI.cyan}${ev.name || ''}${ANSI.reset}(${ANSI.gray}${truncate(ev.args || '', 80)}${ANSI.reset})`);
          } else if (ev.type === 'tool_done') {
            term.writeln(`  ${ANSI.green}✓${ANSI.reset} ${ANSI.gray}${truncate(ev.output || '', 100)}${ANSI.reset}`);
          } else if (ev.type === 'meta') {
            if (assistantStarted) { term.write('\r\n'); assistantStarted = false; }
            const cost = (ev.cost_usd || 0).toFixed(4);
            const elapsed = (ev.elapsed_secs || 0).toFixed(1);
            term.writeln(`${ANSI.gray}[↑ $${cost}  ${elapsed}s]${ANSI.reset}`);
            metaPrinted = true;
          } else if (ev.type === 'error') {
            term.writeln(`${ANSI.red}error:${ANSI.reset} ${ev.message || 'unknown'}`);
          }
        } catch (_) {}
      }
    }
  } catch (err) {
    term.writeln(`${ANSI.red}error:${ANSI.reset} ${err.message || 'network'}`);
  }
  if (assistantStarted) term.write('\r\n');
  if (!metaPrinted) term.writeln('');
  term.writeln('');
  setBusy(false);
  termInput.focus();
});

function truncate(s, n) {
  s = String(s).replace(/\n/g, ' ');
  return s.length > n ? s.slice(0, n) + '…' : s;
}

// ─── Settings / onboarding ───────────────────────────────────────
$('settings-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const status = $('settings-status');
  status.className = 'form-status';
  status.textContent = 'saving…';

  const body = {
    groq_api_key:      $('groq-key').value.trim(),
    gemini_api_key:    $('gemini-key').value.trim(),
    anthropic_api_key: $('anthropic-key').value.trim(),
    cluster_secret:    $('cluster-secret').value.trim(),
  };

  if (!body.groq_api_key && !body.gemini_api_key && !body.anthropic_api_key) {
    status.className = 'form-status err';
    status.textContent = 'set at least one provider key';
    return;
  }

  try {
    const res = await fetch('/api/onboarding', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (res.ok) {
      status.className = 'form-status ok';
      status.textContent = '✓ saved — restart spectyn to apply';
      setTimeout(refreshStatus, 500);
    } else {
      const txt = await res.text();
      status.className = 'form-status err';
      status.textContent = `error: ${txt || res.status}`;
    }
  } catch (err) {
    status.className = 'form-status err';
    status.textContent = `error: ${err.message}`;
  }
});

// ─── Info panels (Todo / Sessions / Cost) ────────────────────────
function getActiveSubtab() {
  const el = document.querySelector('.subtab.active');
  return el ? el.dataset.subtab : 'todo';
}

function refreshInfoActive() {
  const pane = document.getElementById('pane-info');
  if (!pane || !pane.classList.contains('active')) return;
  const sub = getActiveSubtab();
  if (sub === 'todo') refreshTodos();
  else if (sub === 'sessions') refreshSessions();
  else if (sub === 'tools') refreshTools();
  else if (sub === 'cost') refreshCost();
}

function relTime(ts) {
  if (!ts) return '—';
  const sec = Math.max(0, Math.floor(Date.now() / 1000 - Number(ts)));
  if (sec < 60)   return `${sec}s ago`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

function fmtBytes(n) {
  n = Number(n) || 0;
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

async function refreshTodos() {
  const host = document.getElementById('subpane-todo');
  try {
    const res = await fetch('/api/todos');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const todos = await res.json();
    if (!Array.isArray(todos) || todos.length === 0) {
      host.innerHTML = '<div class="info-empty muted">no todos yet — add items to ~/.spectyn-mesh/todos.json</div>';
      return;
    }
    host.innerHTML = todos.map(t => {
      const status = (t.status || 'pending').toLowerCase();
      const cls = ['pending', 'in_progress', 'done'].includes(status) ? status : 'pending';
      const textCls = cls === 'done' ? 'todo-text done' : 'todo-text';
      return `<div class="info-row">
        <span class="todo-dot ${cls}"></span>
        <span class="${textCls}">${escapeHtml(t.text || '')}</span>
        <span class="todo-time">${escapeHtml(relTime(t.created))}</span>
      </div>`;
    }).join('');
  } catch (e) {
    host.innerHTML = `<div class="info-empty muted">error: ${escapeHtml(e.message)}</div>`;
  }
}

async function refreshSessions() {
  const host = document.getElementById('subpane-sessions');
  try {
    const res = await fetch('/api/sessions');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const sessions = await res.json();
    if (!Array.isArray(sessions) || sessions.length === 0) {
      host.innerHTML = '<div class="info-empty muted">no sessions yet — start a chat to create one</div>';
      return;
    }
    host.innerHTML = sessions.map(s => {
      const id = String(s.id || '').slice(0, 12);
      return `<div class="info-row">
        <span class="session-id">${escapeHtml(id)}</span>
        <span class="session-meta">${escapeHtml(fmtBytes(s.size_bytes))}</span>
        <span class="session-time">${escapeHtml(relTime(s.modified))}</span>
        <span class="session-count">${Number(s.message_count || 0)} msgs</span>
      </div>`;
    }).join('');
  } catch (e) {
    host.innerHTML = `<div class="info-empty muted">error: ${escapeHtml(e.message)}</div>`;
  }
}

async function refreshTools() {
  const host = document.getElementById('subpane-tools');
  try {
    const res = await fetch('/api/tools/history');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const tools = await res.json();
    if (!Array.isArray(tools) || tools.length === 0) {
      host.innerHTML = '<div class="info-empty muted">no tool calls yet — run a chat that uses tools (shell, file_read, etc.)</div>';
      return;
    }
    // Newest first.
    tools.sort((a, b) => Number(b.n || 0) - Number(a.n || 0));
    host.innerHTML = tools.map(t => {
      const n         = Number(t.n || 0);
      const name      = String(t.name || '');
      const args      = String(t.args || '');
      const output    = String(t.output || '');
      const argsTrim  = args.length > 60 ? args.slice(0, 60) + '…' : args;
      const lineCount = output ? output.split('\n').length : 0;
      const elapsed   = Number(t.elapsed_ms || 0);
      return `<div class="tool-entry" data-n="${n}">
        <div class="info-row tool-row">
          <span class="tool-tag">[#${n}]</span>
          <span class="tool-name">${escapeHtml(name)}</span>
          <span class="tool-args">${escapeHtml(argsTrim)}</span>
          <span class="tool-meta">${lineCount}L · ${elapsed}ms</span>
        </div>
        <pre class="tool-output" hidden>${escapeHtml(output)}</pre>
      </div>`;
    }).join('');
    // Click → expand/collapse.
    host.querySelectorAll('.tool-row').forEach(row => {
      row.addEventListener('click', () => {
        const pre = row.parentElement.querySelector('.tool-output');
        if (!pre) return;
        const open = !pre.hasAttribute('hidden');
        if (open) pre.setAttribute('hidden', '');
        else pre.removeAttribute('hidden');
        row.classList.toggle('expanded', !open);
      });
    });
  } catch (e) {
    host.innerHTML = `<div class="info-empty muted">error: ${escapeHtml(e.message)}</div>`;
  }
}

async function refreshCost() {
  const host = document.getElementById('subpane-cost');
  try {
    const res = await fetch('/api/cost');
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const c = await res.json();
    const session = Number(c.session_usd || 0);
    const total   = Number(c.total_usd || 0);
    const reqs    = Number(c.requests || 0);
    const pt      = Number(c.prompt_tokens || 0);
    const ct      = Number(c.completion_tokens || 0);
    const byp     = Array.isArray(c.by_provider) ? c.by_provider : [];

    const fmtUsd = v => `$${(Number(v) || 0).toFixed(4)}`;
    let html = '<div class="cost-table">';
    html += `<div class="cost-row"><span class="cost-label">session</span>      <span class="cost-value usd">${fmtUsd(session)}</span></div>`;
    html += `<div class="cost-row"><span class="cost-label">lifetime total</span><span class="cost-value usd">${fmtUsd(total)}</span></div>`;
    html += `<div class="cost-row"><span class="cost-label">requests</span>     <span class="cost-value">${reqs}</span></div>`;
    html += `<div class="cost-row"><span class="cost-label">prompt tokens</span><span class="cost-value">${pt.toLocaleString()}</span></div>`;
    html += `<div class="cost-row"><span class="cost-label">completion tokens</span><span class="cost-value">${ct.toLocaleString()}</span></div>`;

    if (byp.length) {
      html += '<div class="cost-section-title">by provider</div>';
      for (const p of byp) {
        html += `<div class="cost-row"><span class="cost-label">${escapeHtml(p.name || '?')}</span>` +
                `<span class="cost-value">${Number(p.requests || 0)} req · <span class="usd">${fmtUsd(p.usd)}</span></span></div>`;
      }
    } else {
      html += '<div class="cost-section-title">by provider</div>';
      html += '<div class="cost-row"><span class="cost-label muted">no usage recorded yet</span><span></span></div>';
    }
    html += '</div>';
    host.innerHTML = html;
  } catch (e) {
    host.innerHTML = `<div class="info-empty muted">error: ${escapeHtml(e.message)}</div>`;
  }
}

// ─── Cmd-K command palette ───────────────────────────────────────
const PALETTE_COMMANDS = [
  { name: 'Terminal',     hint: 'switch tab',   run: () => activateTab('terminal') },
  { name: 'Tasks / Todo', hint: 'info → todo',  run: () => { activateTab('info'); activateSubtab('todo'); } },
  { name: 'Sessions',     hint: 'info → sessions', run: () => { activateTab('info'); activateSubtab('sessions'); } },
  { name: 'Cost',         hint: 'info → cost',  run: () => { activateTab('info'); activateSubtab('cost'); } },
  { name: 'Settings',     hint: 'switch tab',   run: () => activateTab('settings') },
  { name: 'Help',         hint: 'keybindings',  run: () => paletteToggleHelp() },
  { name: 'Reload',       hint: 'window.location.reload()', run: () => window.location.reload() },
];

const palette = {
  overlay: document.getElementById('palette-overlay'),
  input:   document.getElementById('palette-input'),
  list:    document.getElementById('palette-list'),
  help:    document.getElementById('palette-help'),
  selected: 0,
  filtered: PALETTE_COMMANDS.slice(),
};

function paletteOpen() {
  palette.overlay.hidden = false;
  palette.input.value = '';
  palette.selected = 0;
  palette.help.hidden = true;
  paletteRefilter();
  setTimeout(() => palette.input.focus(), 0);
}

function paletteClose() {
  palette.overlay.hidden = true;
}

function paletteToggleHelp() {
  palette.help.hidden = !palette.help.hidden;
}

function paletteRefilter() {
  const q = palette.input.value.trim().toLowerCase();
  palette.filtered = q
    ? PALETTE_COMMANDS.filter(c => c.name.toLowerCase().includes(q))
    : PALETTE_COMMANDS.slice();
  if (palette.selected >= palette.filtered.length) palette.selected = 0;
  paletteRender();
}

function paletteRender() {
  if (palette.filtered.length === 0) {
    palette.list.innerHTML = '<li class="palette-empty">no matching commands</li>';
    return;
  }
  palette.list.innerHTML = palette.filtered.map((c, i) => {
    const sel = i === palette.selected ? ' selected' : '';
    return `<li class="palette-item${sel}" data-idx="${i}">
      <span>${escapeHtml(c.name)}</span>
      <span class="palette-hint">${escapeHtml(c.hint || '')}</span>
    </li>`;
  }).join('');
  palette.list.querySelectorAll('.palette-item').forEach(el => {
    el.addEventListener('click', () => {
      palette.selected = Number(el.dataset.idx);
      paletteRunSelected();
    });
    el.addEventListener('mousemove', () => {
      const idx = Number(el.dataset.idx);
      if (idx !== palette.selected) {
        palette.selected = idx;
        paletteRender();
      }
    });
  });
}

function paletteRunSelected() {
  const cmd = palette.filtered[palette.selected];
  if (!cmd) return;
  // Close first so commands like 'Help' that re-open or toggle work cleanly.
  if (cmd.name === 'Help') {
    cmd.run();
    return;
  }
  paletteClose();
  try { cmd.run(); } catch (_) {}
}

palette.input.addEventListener('input', paletteRefilter);
palette.input.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    e.preventDefault();
    paletteClose();
  } else if (e.key === 'ArrowDown') {
    e.preventDefault();
    if (palette.filtered.length) {
      palette.selected = (palette.selected + 1) % palette.filtered.length;
      paletteRender();
    }
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    if (palette.filtered.length) {
      palette.selected = (palette.selected - 1 + palette.filtered.length) % palette.filtered.length;
      paletteRender();
    }
  } else if (e.key === 'Enter') {
    e.preventDefault();
    paletteRunSelected();
  }
});
palette.overlay.addEventListener('click', (e) => {
  if (e.target === palette.overlay) paletteClose();
});

document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && (e.key === 'k' || e.key === 'K')) {
    e.preventDefault();
    if (palette.overlay.hidden) paletteOpen();
    else paletteClose();
  } else if (e.key === 'Escape' && !palette.overlay.hidden) {
    e.preventDefault();
    paletteClose();
  }
});
