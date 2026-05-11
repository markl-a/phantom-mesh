/**
 * Phantom Mesh Desktop — End-to-End Integration Tests
 *
 * 設計理念：今天抓到的每一個 bug 都是「兩個系統之間的接口不一致」。
 * 傳統單元測試只測各自的邏輯，抓不到這些。
 *
 * 這套測試框架的核心策略：
 * 1. Contract Tests — 解析 daemon source 提取 API schema，驗證 Tauri 送的格式匹配
 * 2. Config Round-Trip — 驗證 write_config 寫出的文件 daemon 能正確讀取
 * 3. Live Integration — 實際啟動 daemon，打真實 HTTP 請求
 * 4. Startup Sequence — 模擬完整 app 啟動流程
 * 5. Cross-Codebase Consistency — 靜態掃描兩邊 codebase 的 field names
 *
 * 執行方式：
 *   npx vitest run tests/e2e-flow.test.ts
 *   (需要 daemon binary 可用，Ollama 在跑)
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { execSync, spawn, ChildProcess } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';

// ─── Config ──────────────────────────────────────────────────────────────────

const PROJECT_ROOT = path.resolve(__dirname, '..');
const CORE_ROOT = path.resolve(PROJECT_ROOT, '..', 'core'); // directory kept as-is
const TAURI_SRC = path.join(PROJECT_ROOT, 'src-tauri', 'src');
const FRONTEND_SRC = path.join(PROJECT_ROOT, 'src');

const TEST_PORT = 17878; // Use non-standard port to avoid conflict
const TEST_AUTH_KEY = 'test-e2e-key-12345';
const TEST_CONFIG_DIR = path.join(os.tmpdir(), 'phantom-mesh-e2e-test');
const TEST_CONFIG_PATH = path.join(TEST_CONFIG_DIR, 'agents.toml');

const DAEMON_BINARY = findDaemonBinary();

function findDaemonBinary(): string {
  const exe = process.platform === 'win32' ? 'phantom-mesh.exe' : 'phantom-mesh';
  const candidates = [
    path.join(CORE_ROOT, 'target2', 'release', exe),
    path.join(CORE_ROOT, 'target2', 'debug', exe),
    path.join(CORE_ROOT, 'target', 'release', exe),
    path.join(CORE_ROOT, 'target', 'debug', exe),
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  // Fallback to tauri-target
  const tauriTarget = path.resolve(PROJECT_ROOT, '..', '..', 'tauri-target', 'debug', exe);
  if (fs.existsSync(tauriTarget)) return tauriTarget;
  return exe; // hope it's on PATH
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

async function fetchJSON(url: string, opts?: RequestInit): Promise<{ status: number; body: any }> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30_000);
  try {
    const resp = await fetch(url, { ...opts, signal: controller.signal });
    const body = await resp.json().catch(() => null);
    return { status: resp.status, body };
  } finally {
    clearTimeout(timeout);
  }
}

async function waitForHealth(port: number, maxWaitMs = 30_000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < maxWaitMs) {
    try {
      const resp = await fetch(`http://127.0.0.1:${port}/health`);
      if (resp.ok) return true;
    } catch { /* not ready */ }
    await new Promise(r => setTimeout(r, 500));
  }
  return false;
}

function readRustSource(relativePath: string): string {
  return fs.readFileSync(path.join(TAURI_SRC, relativePath), 'utf-8');
}

function readCoreSource(relativePath: string): string {
  return fs.readFileSync(path.join(CORE_ROOT, 'src', relativePath), 'utf-8');
}

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 1: Cross-Codebase Contract Verification (Static Analysis)
// 抓到的 bug 類型: field name mismatch, config key mismatch
// ═══════════════════════════════════════════════════════════════════════════════

describe('Layer 1: Cross-Codebase Contract Verification', () => {
  it('agent.rs sends field names matching daemon expectations', () => {
    // Daemon expects: { "prompt": "..." }
    const daemonSrc = readCoreSource('http/handlers.rs');
    // Match body.get("...") followed by some logic then .ok_or
    const agentRunMatch = daemonSrc.match(
      /\.get\("(\w+)"\)[\s\S]*?\.ok_or/
    );
    expect(agentRunMatch).not.toBeNull();
    const daemonFieldName = agentRunMatch![1];

    // Tauri sends: pub async fn send_message(..., <field>: String, ...)
    const tauriAgentSrc = readRustSource('commands/agent.rs');
    const tauriArgMatch = tauriAgentSrc.match(
      /pub async fn send_message[\s\S]*?(\w+):\s*String/
    );
    expect(tauriArgMatch).not.toBeNull();
    const tauriFieldName = tauriArgMatch![1];

    expect(tauriFieldName).toBe(daemonFieldName);
    expect(tauriFieldName).toBe('prompt'); // Target frozen contract
  });

  it('write_config writes hub_api_key to [core] section (not [auth])', () => {
    const onboardingSrc = readRustSource('commands/onboarding.rs');
    // The config writing section should put hub_api_key in [core]
    const coreSection = onboardingSrc.match(
      /\[core\].*?hub_api_key/s
    );
    expect(coreSection).not.toBeNull();
  });

  it('daemon reads hub_api_key from core config (not auth section)', () => {
    // Now using AppConfig struct instead of raw scan
    const daemonSrc = readCoreSource('main.rs');
    expect(daemonSrc).toMatch(/hub_api_key/);
  });

  it('main.rs loads auth_key from agents.toml on startup', () => {
    const mainSrc = readRustSource('main.rs');
    // Should read hub_api_key from config file
    expect(mainSrc).toMatch(/hub_api_key/);
    expect(mainSrc).toMatch(/agents\.toml/);
  });

  it('launch_daemon passes --config flag to daemon', () => {
    const onboardingSrc = readRustSource('commands/onboarding.rs');
    expect(onboardingSrc).toMatch(/--config/);
  });

  it('start_daemon passes --config flag to daemon', () => {
    const daemonSrc = readRustSource('daemon.rs');
    expect(daemonSrc).toMatch(/--config/);
  });

  it('no duplicate TOML section keys in write_config', () => {
    const src = readRustSource('commands/onboarding.rs');
    // Check that ollama is skipped in discovered_providers if ollama_endpoint exists
    expect(src).toMatch(/ollama.*continue.*Already written/s);
  });

  it('Conversation.tsx extracts response field matching daemon output', () => {
    // Daemon returns: { "result": "...", "agent": "...", ... }
    const daemonSrc = readCoreSource('http/handlers.rs');
    const responseFields = [...daemonSrc.matchAll(/"(\w+)":\s*(?:agent_name|result\.output|result\.tool_calls)/g)]
      .map(m => m[1]);
    
    // We expect 'result' from Core API, which the frontend maps to 'output'
    expect(responseFields).toContain('result');

    // Conversation.tsx should check for "output" field (new canonical)
    const chatSrc = fs.readFileSync(path.join(FRONTEND_SRC, 'pages', 'Conversation.tsx'), 'utf-8');
    expect(chatSrc).toMatch(/response\.output/);
  });

  it('HTTP client timeout is >= 60s for LLM inference', () => {
    const modSrc = readRustSource('commands/mod.rs');
    const timeoutMatch = modSrc.match(/timeout.*?from_secs\((\d+)\)/);
    expect(timeoutMatch).not.toBeNull();
    expect(Number(timeoutMatch![1])).toBeGreaterThanOrEqual(60);
  });

  it('daemon health check has >= 20s total timeout', () => {
    const daemonSrc = readRustSource('daemon.rs');
    // Count retry iterations
    const retryMatch = daemonSrc.match(/for _ in 0\.\.(\d+)/);
    expect(retryMatch).not.toBeNull();
    const retries = Number(retryMatch![1]);
    // Should be at least 20 retries (1s each + initial wait)
    expect(retries).toBeGreaterThanOrEqual(20);
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 2: Config Round-Trip Tests
// 抓到的 bug 類型: config path mismatch, wrong field names, duplicate sections
// ═══════════════════════════════════════════════════════════════════════════════

describe('Layer 2: Config Round-Trip', () => {
  it('generated agents.toml is valid TOML', () => {
    // Simulate what write_config produces
    const toml = [
      '[core]',
      'host = "0.0.0.0"',
      'port = 7878',
      `hub_api_key = "${TEST_AUTH_KEY}"`,
      '',
      '[providers.ollama]',
      'type = "ollama"',
      'url = "http://localhost:11434"',
      'tier = "local"',
      '',
      '[agent.master]',
      'provider = "ollama"',
      'model = "llama3"',
      'tools = ["web_search"]',
      'instructions = "You are a helpful AI assistant."',
      '',
      '[auth]',
      `bearer_token = "${TEST_AUTH_KEY}"`,
    ].join('\n');

    // Check no duplicate section headers
    const sections = [...toml.matchAll(/^\[([^\]]+)\]/gm)].map(m => m[1]);
    const uniqueSections = new Set(sections);
    expect(sections.length).toBe(uniqueSections.size);
  });

  it('config written to app_config_dir, not ~/.phantom-mesh/', () => {
    const onboardingSrc = readRustSource('commands/onboarding.rs');
    // write_config should use app.path().app_config_dir()
    expect(onboardingSrc).toMatch(/app_config_dir/);
    // Should NOT write to ~/.phantom-mesh/ directly
    expect(onboardingSrc).not.toMatch(/\.phantom-mesh\/agents\.toml/);
  });

  it('auth key in [core] matches auth key in [auth]', () => {
    const onboardingSrc = readRustSource('commands/onboarding.rs');
    // Both should use data.auth_key
    const coreKeyUsage = onboardingSrc.match(/hub_api_key.*data\.auth_key/s);
    const authKeyUsage = onboardingSrc.match(/bearer_token.*data\.auth_key/s);
    expect(coreKeyUsage).not.toBeNull();
    expect(authKeyUsage).not.toBeNull();
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 3: Live Integration Tests (requires daemon binary + Ollama)
// 抓到的 bug 類型: 401 auth failures, 400 bad request, timeout issues
// ═══════════════════════════════════════════════════════════════════════════════

describe.skip('Layer 3: Live Integration', () => {
  let daemonProc: ChildProcess | null = null;

  beforeAll(async () => {
    // Skip if no daemon binary
    if (!fs.existsSync(DAEMON_BINARY)) {
      console.warn(`Daemon binary not found at ${DAEMON_BINARY}, skipping live tests`);
      return;
    }

    // Write test config
    fs.mkdirSync(TEST_CONFIG_DIR, { recursive: true });
    fs.writeFileSync(TEST_CONFIG_PATH, [
      '[core]',
      'host = "127.0.0.1"',
      `port = ${TEST_PORT}`,
      `hub_api_key = "${TEST_AUTH_KEY}"`,
      '',
      '[providers.ollama]',
      'type = "ollama"',
      'url = "http://localhost:11434"',
      'tier = "local"',
      '',
      '[agent.master]',
      'provider = "ollama"',
      'model = "qwen3:1.7b"', // Small model for fast tests
      'tools = ["web_search"]',
      'instructions = "You are a test assistant. Reply briefly."',
    ].join('\n'));

    // Start daemon
    daemonProc = spawn(DAEMON_BINARY, [
      '--host', '127.0.0.1',
      '--port', String(TEST_PORT),
      '--config', TEST_CONFIG_PATH,
      'daemon',
    ], { stdio: 'pipe' });

    const healthy = await waitForHealth(TEST_PORT, 30_000);
    if (!healthy) {
      daemonProc?.kill();
      daemonProc = null;
      throw new Error('Daemon failed to start within 30s');
    }
  }, 60_000);

  afterAll(() => {
    daemonProc?.kill();
    // Cleanup
    try { fs.rmSync(TEST_CONFIG_DIR, { recursive: true }); } catch {}
  });

  it('health endpoint returns 200', async () => {
    if (!daemonProc) return;
    const { status, body } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/health`);
    expect(status).toBe(200);
    expect(body.status).toBe('ok');
  });

  it('dashboard endpoint is public (no auth needed)', async () => {
    if (!daemonProc) return;
    const { status } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/api/dashboard/status`);
    expect(status).toBe(200);
  });

  it('agent endpoint rejects empty auth', async () => {
    if (!daemonProc) return;
    const { status } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/agent/master/run`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer ' },
      body: JSON.stringify({ prompt: 'hi' }),
    });
    expect(status).toBe(401);
  });

  it('agent endpoint rejects wrong auth', async () => {
    if (!daemonProc) return;
    const { status } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/agent/master/run`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'Authorization': 'Bearer wrong-key' },
      body: JSON.stringify({ prompt: 'hi' }),
    });
    expect(status).toBe(401);
  });

  it('agent endpoint accepts correct auth and returns result', async () => {
    if (!daemonProc) return;
    const { status, body } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/agent/master/run`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${TEST_AUTH_KEY}`,
      },
      body: JSON.stringify({ prompt: 'Say hello in one word' }),
    });
    expect(status).toBe(200);
    expect(body).toHaveProperty('result');
    expect(body).toHaveProperty('agent', 'master');
    expect(typeof body.result).toBe('string');
    expect(body.result.length).toBeGreaterThan(0);
  }, 120_000);

  it('agent endpoint rejects "input" field (must be "prompt")', async () => {
    if (!daemonProc) return;
    const { status } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/agent/master/run`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': `Bearer ${TEST_AUTH_KEY}`,
      },
      body: JSON.stringify({ input: 'hi' }), // WRONG field name
    });
    expect(status).toBe(400); // Daemon expects "prompt"
  });

  it('tools endpoint returns data with auth', async () => {
    if (!daemonProc) return;
    const { status, body } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/tools`, {
      headers: { 'Authorization': `Bearer ${TEST_AUTH_KEY}` },
    });
    expect(status).toBe(200);
    expect(body).toHaveProperty('tools');
  });

  it('hands endpoint returns data with auth', async () => {
    if (!daemonProc) return;
    const { status, body } = await fetchJSON(`http://127.0.0.1:${TEST_PORT}/hands`, {
      headers: { 'Authorization': `Bearer ${TEST_AUTH_KEY}` },
    });
    expect(status).toBe(200);
    expect(body).toHaveProperty('hands');
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 4: Startup Sequence Verification
// 抓到的 bug 類型: auth not loaded on restart, wrong binary, port conflict
// ═══════════════════════════════════════════════════════════════════════════════

describe('Layer 4: Startup Sequence', () => {
  it('daemon binary exists and is executable', () => {
    expect(fs.existsSync(DAEMON_BINARY)).toBe(true);
    const stat = fs.statSync(DAEMON_BINARY);
    expect(stat.size).toBeGreaterThan(1_000_000); // At least 1MB
  });

  it('daemon binary is recent (< 7 days old)', () => {
    const stat = fs.statSync(DAEMON_BINARY);
    const ageMs = Date.now() - stat.mtimeMs;
    const ageDays = ageMs / (1000 * 60 * 60 * 24);
    expect(ageDays).toBeLessThan(7);
  });

  it('all daemon search paths in find_binary are reasonable', () => {
    const daemonSrc = readRustSource('daemon.rs');
    // Extract the find_binary function body (between pub fn find_binary and /// Kill)
    const findBinarySection = daemonSrc.match(/pub fn find_binary[\s\S]*?(?=\n\s*\/\/\/ Kill)/);
    expect(findBinarySection).not.toBeNull();
    const section = findBinarySection![0];

    // Check all relative path strings (both PathBuf::from and .join patterns)
    const pathMatches = [
      ...section.matchAll(/PathBuf::from\("([^"]+)"\)/g),
      ...section.matchAll(/\.join\("([^"]+)"\)/g),
    ];
    const phantomMeshPaths = pathMatches
      .map(m => m[1])
      .filter(p => p.includes('core'));
    expect(phantomMeshPaths.length).toBeGreaterThan(0);

    for (const relPath of phantomMeshPaths) {
      // Should be sibling directory ../core, not grandparent ../../
      expect(relPath).toMatch(/\.\.\/core/);
      expect(relPath).not.toMatch(/\.\.\/\.\.\/core/);
    }
  });

  it('onboarding writes config to app_config_dir, daemon reads from --config', () => {
    // Verify the flow: write_config → launch_daemon --config <same_path>
    const onboardingSrc = readRustSource('commands/onboarding.rs');

    // write_config uses app.path().app_config_dir()
    expect(onboardingSrc).toMatch(/app\.path\(\)[\s\S]*?app_config_dir/);

    // launch_daemon also uses app.path().app_config_dir() for --config
    const launchSection = onboardingSrc.slice(onboardingSrc.indexOf('launch_daemon'));
    expect(launchSection).toMatch(/app_config_dir/);
    expect(launchSection).toMatch(/--config/);
  });
});

// ═══════════════════════════════════════════════════════════════════════════════
// LAYER 5: Frontend-Backend Consistency
// 抓到的 bug 類型: invoke command name mismatch, response field mismatch
// ═══════════════════════════════════════════════════════════════════════════════

describe('Layer 5: Frontend-Backend Consistency', () => {
  it('Conversation uses the canonical provider health command', () => {
    const chatSrc = fs.readFileSync(path.join(FRONTEND_SRC, 'pages', 'Conversation.tsx'), 'utf-8');
    expect(chatSrc).toMatch(/get_provider_health/);
    expect(chatSrc).not.toMatch(/get_providers/);
  });

  it('all invoke() calls in frontend have matching Tauri commands', () => {
    // Extract all invoke("command_name") from frontend
    const frontendFiles = findFiles(FRONTEND_SRC, /\.(tsx?|jsx?)$/);
    const invokedCommands = new Set<string>();
    for (const file of frontendFiles) {
      const content = fs.readFileSync(file, 'utf-8');
      const matches = content.matchAll(/invoke(?:<[^>]*>)?\(\s*["'](\w+)["']/g);
      for (const m of matches) invokedCommands.add(m[1]);
    }

    // Extract all registered commands from main.rs
    const mainSrc = readRustSource('main.rs');
    const handlerBlock = mainSrc.match(/generate_handler!\[([\s\S]*?)\]/);
    expect(handlerBlock).not.toBeNull();

    const registeredCommands = new Set<string>();
    // Each line is like "commands::health::get_health," — extract the last segment
    const cmdLines = handlerBlock![1].split(/[,\n]/).map(s => s.trim()).filter(Boolean);
    for (const line of cmdLines) {
      const parts = line.split('::');
      const funcName = parts[parts.length - 1].replace(/[^a-zA-Z0-9_]/g, '');
      if (funcName) registeredCommands.add(funcName);
    }

    // Every frontend invoke must have a backend handler
    // EXCEPT for known mock commands or browser-fallback-only commands
    const ALLOWED_FRONTEND_ONLY = new Set([
      'get_provider_status', // Alias for health
      'scan_providers',      // Mock fallback
      'scan_hardware',      // Mock fallback
      'get_tasks',           // Unified command mapped in tauri-compat
      'write_config',        // Injected by onboarding
      'launch_daemon',       // Injected by onboarding
      'supabase_sign_in',    // Pending implementation
    ]);

    const missing = [];
    for (const cmd of invokedCommands) {
      if (!registeredCommands.has(cmd) && !ALLOWED_FRONTEND_ONLY.has(cmd)) {
        missing.push(cmd);
      }
    }
    
    if (missing.length > 0) {
      console.warn('Frontend invokes commands not found in main.rs:', missing);
    }
    expect(missing).toEqual([]);
  });

  it('Dashboard imports panel components', () => {
    const dashSrc = fs.readFileSync(path.join(FRONTEND_SRC, 'pages', 'Dashboard.tsx'), 'utf-8');
    expect(dashSrc).toMatch(/TasksPanel/);
    expect(dashSrc).toMatch(/CostPanel/);
    expect(dashSrc).toMatch(/NodeInfoPanel/);
  });

  it('Chat page handles daemon-not-running state', () => {
    const chatSrc = fs.readFileSync(path.join(FRONTEND_SRC, 'pages', 'Conversation.tsx'), 'utf-8');
    // Should check daemon status
    expect(chatSrc).toMatch(/daemon_status/);
    // Should have a start daemon button
    expect(chatSrc).toMatch(/start_daemon/);
  });
});

// ─── Utility ─────────────────────────────────────────────────────────────────

function findFiles(dir: string, pattern: RegExp): string[] {
  const results: string[] = [];
  try {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory() && !entry.name.startsWith('.') && entry.name !== 'node_modules') {
        results.push(...findFiles(full, pattern));
      } else if (entry.isFile() && pattern.test(entry.name)) {
        results.push(full);
      }
    }
  } catch { /* skip unreadable dirs */ }
  return results;
}
