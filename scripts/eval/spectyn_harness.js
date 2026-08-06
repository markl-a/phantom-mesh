// spectyn_harness.js — deterministic, OFFLINE provider harness for promptfoo.
//
// This is a promptfoo custom JS provider (referenced as `file://spectyn_harness.js`).
// It does NOT call any LLM and never touches the network. Instead it drives the
// local `spectyn` CLI (or a stubbed binary via SPECTYN_BIN) for commands that
// produce stable, deterministic output, then returns that text so promptfoo's
// non-LLM asserts (contains / regex / is-json / javascript) can validate
// spectyn's own behavior.
//
// promptfoo loads this file as a class with `id()` + `callApi()` (the stable
// custom-provider interface). The `vars.cmd` of each test row selects which
// deterministic probe to run:
//   - "version"   → `spectyn --version`           (build/version banner)
//   - "help"      → `spectyn --help`               (top-level command surface)
//   - "exec-help" → `spectyn exec --help`          (headless run contract)
//   - "route"     → pure in-process provider-routing parse (NO subprocess, NO net)
//
// ANSI color codes are stripped so asserts match plain text on any terminal.

const { spawnSync } = require('node:child_process');

const SPECTYN_BIN = process.env.SPECTYN_BIN || 'spectyn';

// Strip ANSI SGR sequences so assertions are terminal-independent.
function stripAnsi(s) {
  // eslint-disable-next-line no-control-regex
  return String(s).replace(/\x1b\[[0-9;]*m/g, '');
}

function runSpectyn(args) {
  const r = spawnSync(SPECTYN_BIN, args, {
    encoding: 'utf8',
    timeout: 20000,
    stdio: ['ignore', 'pipe', 'pipe'],
    // Keep it offline + reproducible: no implicit color, no implicit network.
    env: { ...process.env, NO_COLOR: '1' },
  });
  if (r.error) {
    // ENOENT (binary missing) etc.
    return { ok: false, text: String(r.error.message || r.error) };
  }
  // spectyn writes --help / --version banners to STDERR (and exits 0), while
  // some output lands on STDOUT. Merge both streams so asserts see all text.
  const merged = stripAnsi((r.stdout || '') + (r.stderr || ''));
  if (merged.trim().length === 0) {
    return { ok: false, text: `empty output (status ${r.status})` };
  }
  return { ok: true, text: merged };
}

// Pure, dependency-free model of spectyn's provider:model precedence rule
// (documented project behavior: agent.X.model overrides providers.X.default_model,
//  and "provider:model" syntax pins a specific model on a provider).
// This case asserts prompt/route CONSTRUCTION logic with zero I/O.
function resolveRoute(spec) {
  // spec like "groq:llama-3.1-8b-instant" or just "groq"
  const [provider, ...rest] = String(spec).split(':');
  const model = rest.length ? rest.join(':') : null;
  return {
    provider,
    model, // null => provider default_model is used
    pinned: model !== null,
  };
}

class SpectynOfflineProvider {
  constructor(options) {
    this.options = options || {};
    this.providerId =
      (this.options.id && String(this.options.id)) || 'spectyn-offline-harness';
  }

  id() {
    return this.providerId;
  }

  // promptfoo custom-provider entry point.
  // `prompt` is the rendered prompt string; `context.vars` holds the row vars.
  async callApi(prompt, context) {
    const vars = (context && context.vars) || {};
    const cmd = vars.cmd || 'version';

    if (cmd === 'route') {
      const route = resolveRoute(vars.spec || 'groq:llama-3.1-8b-instant');
      return { output: JSON.stringify(route) };
    }

    let args;
    switch (cmd) {
      case 'version':
        args = ['--version'];
        break;
      case 'help':
        args = ['--help'];
        break;
      case 'exec-help':
        args = ['exec', '--help'];
        break;
      default:
        return { error: `unknown cmd var: ${cmd}` };
    }

    const res = runSpectyn(args);
    if (!res.ok) {
      return { error: `spectyn ${args.join(' ')} failed: ${res.text}` };
    }
    return { output: res.text };
  }
}

module.exports = SpectynOfflineProvider;
// Also expose pure helpers for unit-style reuse / inspection.
module.exports.resolveRoute = resolveRoute;
module.exports.stripAnsi = stripAnsi;
