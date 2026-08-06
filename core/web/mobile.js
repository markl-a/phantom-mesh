/* spectyn mobile — minimal SSE chat client.
 *
 * Talks to the same /api/chat endpoint as the desktop UI. Each chunk is a
 * `data: {json}\n` line; events: token / thinking / tool_start / tool_done
 * / done / error. We render each into a styled <div class="mmsg ..."> bubble.
 */
(() => {
  const $ = (id) => document.getElementById(id);
  const screen  = $('m-screen');
  const welcome = $('m-welcome');
  const status  = $('m-status');
  const input   = $('m-input');
  const send    = $('m-send');
  const cancel  = $('m-cancel');
  const form    = $('m-form');

  let abortCtrl = null;
  let assistantBubble = null;
  let toolStartedAt = null;

  const setStatus = (cls, title) => {
    status.className = 'dot ' + (cls || '');
    if (title) status.title = title;
  };

  const scrollEnd = () => {
    requestAnimationFrame(() => {
      screen.scrollTop = screen.scrollHeight;
    });
  };

  const addMsg = (cls, text) => {
    if (welcome && welcome.parentNode) welcome.remove();
    const el = document.createElement('div');
    el.className = 'mmsg ' + cls;
    if (text != null) el.textContent = text;
    screen.appendChild(el);
    scrollEnd();
    return el;
  };

  const setBusy = (busy) => {
    send.disabled = busy;
    cancel.hidden = !busy;
    setStatus(busy ? 'busy' : 'ok', busy ? 'streaming…' : 'ready');
  };

  const autoResize = () => {
    input.style.height = 'auto';
    input.style.height = Math.min(input.scrollHeight, 120) + 'px';
  };

  // ── send a turn ────────────────────────────────────────────────────────
  async function sendTurn(prompt) {
    if (!prompt.trim()) return;
    addMsg('user', prompt);
    input.value = '';
    autoResize();
    setBusy(true);
    assistantBubble = null;
    toolStartedAt = null;

    abortCtrl = new AbortController();
    let res;
    try {
      res = await fetch('/api/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ prompt }),
        signal: abortCtrl.signal,
      });
    } catch (e) {
      addMsg('error', String(e && e.message ? e.message : e));
      setBusy(false);
      return;
    }

    if (!res.ok) {
      addMsg('error', `HTTP ${res.status} — ${await res.text().catch(() => '')}`);
      setBusy(false);
      return;
    }

    const reader = res.body.getReader();
    const dec = new TextDecoder();
    let buf = '';

    try {
      while (true) {
        const { value, done } = await reader.read();
        if (done) break;
        buf += dec.decode(value, { stream: true });
        const lines = buf.split('\n');
        buf = lines.pop() || '';
        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          const json = line.slice(6).trim();
          if (!json) continue;
          let ev;
          try { ev = JSON.parse(json); } catch { continue; }
          handleEvent(ev);
        }
      }
    } catch (e) {
      if (e.name !== 'AbortError') addMsg('error', String(e.message || e));
    } finally {
      setBusy(false);
      abortCtrl = null;
    }
  }

  function handleEvent(ev) {
    const type = ev.type;
    if (type === 'token') {
      if (!assistantBubble) assistantBubble = addMsg('assistant', '');
      assistantBubble.textContent += (ev.content || '');
      scrollEnd();
    } else if (type === 'thinking') {
      // collect thinking into its own bubble (single one until next token)
      let bubble = screen.lastElementChild;
      if (!bubble || !bubble.classList.contains('thinking')) {
        bubble = addMsg('thinking', '');
      }
      bubble.textContent += (ev.content || '');
      scrollEnd();
    } else if (type === 'tool_start') {
      toolStartedAt = Date.now();
      const args = (ev.args || '').slice(0, 80);
      addMsg('tool', `${ev.name || 'tool'}(${args})`);
      assistantBubble = null;
    } else if (type === 'tool_done') {
      const dur = toolStartedAt ? ((Date.now() - toolStartedAt) / 1000).toFixed(1) : '?';
      const out = (ev.output || '').replace(/\s+/g, ' ').slice(0, 120);
      addMsg('tool', `↳ ${dur}s · ${out || 'ok'}`);
      assistantBubble = null;
    } else if (type === 'done') {
      const meta = [];
      if (ev.cost_usd != null) meta.push(`$${Number(ev.cost_usd).toFixed(4)}`);
      if (ev.elapsed_secs != null) meta.push(`${Number(ev.elapsed_secs).toFixed(1)}s`);
      if (meta.length) addMsg('done', meta.join(' · '));
    } else if (type === 'error') {
      addMsg('error', ev.message || 'unknown error');
    }
  }

  // ── form events ────────────────────────────────────────────────────────
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    sendTurn(input.value);
  });

  input.addEventListener('input', autoResize);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.altKey) {
      // Desktop browser convenience; on mobile virtual keyboard the Enter
      // usually inserts newline anyway, so we keep Shift+Enter behavior.
      if (window.matchMedia('(hover: hover)').matches) {
        e.preventDefault();
        sendTurn(input.value);
      }
    }
  });

  // ── modbar actions ─────────────────────────────────────────────────────
  document.querySelectorAll('.mmod').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.preventDefault();
      const ins = btn.dataset.insert;
      const key = btn.dataset.key;
      const action = btn.dataset.action;
      if (action === 'cancel' && abortCtrl) {
        abortCtrl.abort();
        addMsg('error', '— cancelled —');
        return;
      }
      input.focus();
      if (ins) {
        const s = input.selectionStart, e2 = input.selectionEnd;
        const v = input.value;
        input.value = v.slice(0, s) + ins + v.slice(e2);
        input.selectionStart = input.selectionEnd = s + ins.length;
        autoResize();
      } else if (key === 'Tab') {
        const s = input.selectionStart, e2 = input.selectionEnd;
        const v = input.value;
        input.value = v.slice(0, s) + '\t' + v.slice(e2);
        input.selectionStart = input.selectionEnd = s + 1;
      } else if (key === 'ArrowUp' || key === 'ArrowDown') {
        // future: history navigation. PoC stub.
      }
    });
  });

  // ── status check on load ───────────────────────────────────────────────
  fetch('/healthz').then((r) => {
    setStatus(r.ok ? 'ok' : 'err', r.ok ? 'connected' : 'unhealthy');
  }).catch(() => setStatus('err', 'offline'));

  setStatus('', 'ready');
})();
