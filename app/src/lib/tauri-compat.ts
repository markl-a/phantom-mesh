let _invoke: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null = null;

const DAEMON = "http://localhost:7878";

export function isTauri(): boolean {
  // Tauri 2 mobile injects only `__TAURI_INTERNALS__`; v1/desktop also
  // expose `__TAURI__`. Checking just `__TAURI__` mis-detects mobile as
  // browser, dropping every invoke() into the HTTP fallback (write_config
  // becomes a no-op, scan_hardware returns dummy zeros).
  if (typeof window === "undefined") return false;
  const w = window as unknown as { __TAURI__?: unknown; __TAURI_INTERNALS__?: unknown };
  return w.__TAURI_INTERNALS__ !== undefined || w.__TAURI__ !== undefined;
}

async function getInvoke() {
  if (_invoke) return _invoke;
  if (isTauri()) {
    const mod = await import("@tauri-apps/api/core");
    _invoke = mod.invoke;
  } else {
    _invoke = httpFallback;
  }
  return _invoke;
}

async function httpFallback(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    // ── Conversation ──
    case "send_message": {
      const agent = (args?.agent as string) || "master";
      const body: Record<string, unknown> = { prompt: args?.prompt };
      if (args?.chat_id) body.chat_id = args.chat_id;
      const resp = await post(`/agent/${agent}/run`, body);
      return resp;
    }
    case "get_conversations":
      return get("/conversations/history").catch(() => ({ messages: [] }));
    case "list_conversations":
      return get("/conversations/list").catch(() => ({ conversations: [] }));
    case "get_conversation_history":
      return get(`/conversations/${args?.chat_id ?? "daemon"}/history`).catch(() => ({ messages: [] }));
    case "reset_conversation":
      return post(`/conversations/${args?.chat_id ?? "daemon"}/reset`, {}).catch(() => ({}));

    // ── Health / Status ──
    case "get_health":
    case "daemon_status":
      return get("/health").then((h: Record<string, unknown>) => ({ ...h, healthy: h["status"] === "ok" }));
    case "get_version":
      return get("/api/version").catch(() => ({ version: "unknown", commit: "" }));
    case "get_peers":
      return get("/rpc/peers").catch(() => ({ peers: [], self: { name: "", version: "" } }));
    case "start_daemon":
      return { ok: true };
    case "get_estop_status":
      return { stopped: false };

    // ── Dashboard ──
    case "get_dashboard_status":
      return get("/api/dashboard/status");
    case "get_cluster_status":
      return get("/cluster/status").catch(() => ({ nodes: [] }));
    case "get_cluster_workers":
      return get("/cluster/workers").catch(() => ({ workers: [] }));
    case "get_cluster_scores":
      return get("/cluster/scores").catch(() => ({ scores: [] }));
    case "get_provider_health":
      return get("/api/providers/health");
    case "get_costs":
      return get("/costs").catch(() => ({ total_usd: 0 }));
    case "get_revenue":
      return { total_usd: 0 };
    case "get_task_history":
    case "get_tasks":
      return get("/task/history").catch(() => ({ tasks: [] }));

    // ── Tools / Hands ──
    case "get_tools":
      return get("/tools");
    case "get_hands":
      return get("/hands");

    // ── Memory ──
    case "get_memory_observations":
      return get("/memory/observations").catch(() => ({ observations: [] }));
    case "get_memory_stats":
      return get("/memory/observations/stats").catch(() => ({ total_observations: 0 }));
    case "search_memory":
      return { observations: [] };

    // ── Security / Audit ──
    case "get_audit_log":
      return get("/audit").catch(() => ({ entries: [] }));

    // ── Networking ──
    case "get_network_discovery":
      return { discovered: 0 };
    case "get_network_routes":
      return { discovery_count: 0, transport_count: 0 };
    case "get_network_status":
      return { status: "ok", mode: "browser" };

    // ── Onboarding (detect local machine via daemon) ──
    case "scan_hardware":
      return get("/scan/hardware").catch(() => ({
        gpu: "Unknown", vram_mb: 0, gpus: [], npus: [],
        ram_mb: 0, ollama_status: "offline", ollama_models: [],
        daemon_binary_path: null, available_port: 7878,
      }));
    case "scan_credentials":
      return get("/scan/credentials").catch(() => []);
    case "write_config":
      return {};
    case "launch_daemon":
      return { ok: true, pid: null, port: 7878 };
    case "validate_api_key":
      return { ok: false, models: [], error: "Browser mode" };
    case "generate_qr_data":
      return { type: "phantom-mesh-hub", version: 1, hub_url: "", auth_key: "", node_id: "" };
    case "get_local_ip":
      return "127.0.0.1";
    case "open_external_url":
      if (args?.url) window.open(args.url as string, "_blank");
      return {};

    // ── OAuth — open browser, poll result (handled by OnboardingQuickStart directly) ──
    case "oauth_sign_in":
      throw new Error("OAuth is handled directly by the onboarding component via HTTP.");

    // ── Settings ──
    case "get_config":
      return {};
    case "set_config":
      return {};

    // ── Supabase ──
    case "supabase_sign_in":
    case "supabase_get_session":
    case "supabase_log_usage":
    case "supabase_backup_config":
    case "supabase_restore_config":
    case "supabase_sign_out":
      return {};

    // ── Goals ──
    case "goals_list":
      return [];
    case "goals_create":
    case "goals_get":
    case "goals_update":
    case "goals_delete":
    case "goals_progress":
    case "goals_today":
    case "goals_summary":
    case "goals_milestones":
    case "goals_milestone_add":
    case "goals_milestone_toggle":
    case "goals_recurring_tasks":
    case "goals_recurring_add":
    case "goals_recurring_complete":
    case "goals_checkin_add":
    case "goals_checkins":
    case "goals_mood_trend":
    case "goals_weekly_summary":
    case "goals_global_mood":
      return {};

    // ── Browser / Pages ──
    case "browser_navigate":
    case "browser_screenshot":
    case "browser_snapshot":
    case "browser_status":
    case "browser_close":
    case "list_pages":
    case "load_page":
    case "save_page":
    case "delete_page":
    case "page_db_get":
    case "page_db_set":
    case "page_db_query":
      return {};

    // ── Updater ──
    case "check_for_updates":
      return { available: false };
    case "install_update":
      return {};

    // ── Copilot / Claude tokens ──
    case "read_copilot_token":
      return { found: false, user: null };
    case "read_gcloud_adc":
      return { found: false, project: null };
    case "read_claude_cli_token":
      return { found: false };

    default:
      console.warn(`[tauri-compat] Unknown command: ${cmd}`);
      return {};
  }
}

async function get(path: string): Promise<any> {
  const resp = await fetch(`${DAEMON}${path}`);
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

async function post(path: string, body: unknown): Promise<any> {
  const resp = await fetch(`${DAEMON}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
  return resp.json();
}

export async function safeInvoke<T = unknown>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const fn = await getInvoke();
  return fn!(command, args) as Promise<T>;
}
