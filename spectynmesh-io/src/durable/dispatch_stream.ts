// F205 — DispatchStream Durable Object.
//
// One DO instance per dispatch job_id. The instance owns the in-memory
// chunk buffer + the set of active SSE subscribers (browser tabs that
// opened an EventSource against /api/me/dispatch/stream/:job_id).
//
// CONTROL-PLANE MODEL (deviates from F205 spec text — documented in PR body):
//   The user prompt explicitly says: "the worker is a CONTROL PLANE that
//   talks to the user's local daemon via the user's browser (XHR direct
//   from SPA to localhost with `spectyn serve --cors=…`)". So the DO
//   does NOT proxy upstream to spectyn serve — that would route the
//   user's localhost traffic through Cloudflare, which the directive
//   forbids.
//
//   Instead the SPA (which is talking to localhost directly) PUSHES each
//   chunk it receives locally to the DO via `POST /api/me/dispatch/stream/:job_id`,
//   and any OTHER browser tab subscribed via `GET /api/me/dispatch/stream/:job_id`
//   receives the same chunks. This solves the "two tabs see identical
//   sequences" acceptance criterion without putting upstream LLM traffic
//   on the worker's billable bandwidth.
//
// Persistence: the chunk buffer lives in DO storage so a fresh subscriber
// joining mid-stream replays everything sent so far. After the SPA
// publishes a terminal event (status=done|cancelled|error), the DO sets
// a 60-second alarm and self-deletes its storage on alarm fire — the
// final transcript already lives in the dispatches D1 row written by
// the /start handler.

export interface DispatchChunk {
  /// Monotonic id, increments per chunk published to this instance.
  seq: number;
  /// Type — "chunk" carries text, "status" carries lifecycle transitions,
  /// "error" carries a terminal failure message.
  kind: "chunk" | "status" | "error";
  /// Payload — for "chunk" this is the text fragment; for "status" it's
  /// one of "running" | "done" | "cancelled"; for "error" it's the
  /// human-readable message.
  data: string;
  /// Server-set unix ms when the chunk was accepted.
  ts: number;
}

interface DispatchState {
  /// Owner user_id — set on first publish. Subscribers must match (we
  /// re-check inside the DO so a leaked DO id can't be used to subscribe
  /// to another user's job).
  user_id: number | null;
  /// Job id — redundant with the DO instance name but useful for logs.
  job_id: string;
  /// Whether the stream has reached a terminal state (done/cancelled/error).
  closed: boolean;
  /// Monotonic chunk counter.
  next_seq: number;
  /// All chunks received so far. Capped at 5_000 chunks to bound memory;
  /// older chunks fall off (extremely long dispatches lose replay history,
  /// but the live tail and the final D1 transcript are unaffected).
  buffer: DispatchChunk[];
}

/// Maximum chunks held in replay buffer per DO instance. Beyond this the
/// oldest entries drop off — a late-joining tab will miss the head but
/// see all subsequent chunks. The full final transcript persists in D1.
const MAX_BUFFER = 5000;

/// Subscriber cap (F205 spec §Scope "Out": cap at 5 simultaneous viewers).
/// 6th subscriber gets a 429 response.
const MAX_SUBSCRIBERS = 5;

/// How long to keep the DO alive after the terminal event so latecomers
/// can still replay. After this elapses the alarm fires and storage is
/// purged.
const EVICTION_MS = 60_000;

interface Subscriber {
  user_id: number;
  controller: ReadableStreamDefaultController<Uint8Array>;
  closed: boolean;
}

/// Note on environment shape — the DO does NOT need the full Worker `Env`
/// (no DB / KV access from inside the DO). Cloudflare passes a minimal
/// env on construction; we only use storage + the WebSocket / Streams
/// primitives.
type DOEnv = Record<string, unknown>;

export class DispatchStream {
  private state: DurableObjectState;
  private encoder = new TextEncoder();
  /// Live SSE subscribers attached via GET /stream.
  private subscribers = new Set<Subscriber>();
  /// Cached state (loaded lazily from storage on first request).
  private mem: DispatchState | null = null;

  constructor(state: DurableObjectState, _env: DOEnv) {
    this.state = state;
  }

  /// Lazy-load DO storage state. Cached after first hit so subsequent
  /// requests don't pay the IO each time.
  private async load(): Promise<DispatchState> {
    if (this.mem !== null) return this.mem;
    const fromDisk = await this.state.storage.get<DispatchState>("state");
    this.mem = fromDisk ?? {
      user_id: null,
      job_id: "",
      closed: false,
      next_seq: 1,
      buffer: [],
    };
    return this.mem;
  }

  /// Persist current state — call after every mutation so an eviction
  /// (DO recycled by Cloudflare) doesn't lose the buffer.
  private async persist(): Promise<void> {
    if (this.mem === null) return;
    await this.state.storage.put("state", this.mem);
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const method = request.method.toUpperCase();
    // Worker-side caller MUST attach `X-Owner` (user_id from the verified
    // JWT) before forwarding the request to the DO. The DO refuses any
    // request without this header — defense in depth so a leaked DO
    // namespace id can't be used directly from outside the worker.
    const ownerHdr = request.headers.get("X-Owner") ?? "";
    const owner = Number.parseInt(ownerHdr, 10);
    if (!Number.isFinite(owner) || owner <= 0) {
      return new Response(JSON.stringify({ error: "missing X-Owner" }), {
        status: 400, headers: { "Content-Type": "application/json" },
      });
    }
    const mem = await this.load();
    // First request claims ownership. Subsequent owners must match.
    if (mem.user_id === null) {
      mem.user_id = owner;
      const j = url.searchParams.get("job_id") ?? "";
      if (j) mem.job_id = j;
      await this.persist();
    } else if (mem.user_id !== owner) {
      // Cross-user attempt — refuse silently with 404 so we don't
      // confirm the existence of another user's job.
      return new Response(JSON.stringify({ error: "not found" }), {
        status: 404, headers: { "Content-Type": "application/json" },
      });
    }

    if (method === "POST" && url.pathname.endsWith("/publish")) {
      return this.handlePublish(request, mem);
    }
    if (method === "GET" && url.pathname.endsWith("/subscribe")) {
      return this.handleSubscribe(owner, mem);
    }
    if (method === "POST" && url.pathname.endsWith("/cancel")) {
      return this.handleCancel(mem);
    }
    if (method === "GET" && url.pathname.endsWith("/snapshot")) {
      return new Response(JSON.stringify({
        job_id: mem.job_id,
        closed: mem.closed,
        chunk_count: mem.buffer.length,
        subscribers: this.subscribers.size,
      }), { headers: { "Content-Type": "application/json" } });
    }
    return new Response(JSON.stringify({ error: "unknown DO route" }), {
      status: 404, headers: { "Content-Type": "application/json" },
    });
  }

  /// SPA POSTs each locally-received chunk here so other tabs see it.
  /// Body: { kind: "chunk"|"status"|"error", data: string }.
  /// Returns: { seq: <assigned> }.
  private async handlePublish(request: Request, mem: DispatchState): Promise<Response> {
    if (mem.closed) {
      // Idempotent — extra publishes after terminal are accepted but
      // routed only to currently-attached subscribers (replay buffer
      // already truncated for eviction). We choose 409 so the caller
      // knows the stream was already finalized.
      return new Response(JSON.stringify({ error: "stream closed" }), {
        status: 409, headers: { "Content-Type": "application/json" },
      });
    }
    let body: { kind?: string; data?: string };
    try {
      body = await request.json() as { kind?: string; data?: string };
    } catch {
      return new Response(JSON.stringify({ error: "malformed json" }), {
        status: 400, headers: { "Content-Type": "application/json" },
      });
    }
    const kind = body.kind === "status" || body.kind === "error" ? body.kind : "chunk";
    const data = typeof body.data === "string" ? body.data : "";

    const chunk: DispatchChunk = {
      seq: mem.next_seq++,
      kind: kind as DispatchChunk["kind"],
      data,
      ts: Date.now(),
    };
    mem.buffer.push(chunk);
    if (mem.buffer.length > MAX_BUFFER) {
      // Drop from the front; live subscribers already saw these.
      mem.buffer.splice(0, mem.buffer.length - MAX_BUFFER);
    }

    // Terminal events close the stream and schedule eviction.
    if (kind === "status" && (data === "done" || data === "cancelled")) {
      mem.closed = true;
      await this.state.storage.setAlarm(Date.now() + EVICTION_MS);
    } else if (kind === "error") {
      mem.closed = true;
      await this.state.storage.setAlarm(Date.now() + EVICTION_MS);
    }

    await this.persist();
    this.fanOut(chunk);

    // If this was a terminal event, also close every subscriber stream
    // gracefully so the client EventSource gets `onerror` and stops
    // retrying after the standard browser back-off.
    if (mem.closed) {
      for (const sub of this.subscribers) this.closeSubscriber(sub);
      this.subscribers.clear();
    }
    return new Response(JSON.stringify({ seq: chunk.seq }), {
      headers: { "Content-Type": "application/json" },
    });
  }

  /// SSE subscriber attach — replay the buffer immediately, then stream
  /// new chunks as they arrive.
  private async handleSubscribe(owner: number, mem: DispatchState): Promise<Response> {
    if (this.subscribers.size >= MAX_SUBSCRIBERS) {
      return new Response(JSON.stringify({ error: "too many subscribers" }), {
        status: 429,
        headers: { "Content-Type": "application/json", "Retry-After": "30" },
      });
    }

    const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>();
    const writer = writable.getWriter();
    let controllerRef: ReadableStreamDefaultController<Uint8Array> | null = null;
    const sub: Subscriber = {
      user_id: owner,
      // We use the writer for actual writes; the controller is unused
      // but the interface keeps a type-safe handle for closeSubscriber.
      controller: null as unknown as ReadableStreamDefaultController<Uint8Array>,
      closed: false,
    };
    // Custom close helper: drop from set + close the writer.
    const closeMe = async () => {
      if (sub.closed) return;
      sub.closed = true;
      this.subscribers.delete(sub);
      try { await writer.close(); } catch { /* already closed */ }
    };
    // Replace the dummy controller with one that writes via the writer.
    // Easier: just keep a reference to the writer on the subscriber by
    // swapping fanOut to handle writers directly. To stay backwards
    // compatible with the simple structure, we wrap the writer into a
    // pseudo-controller-shaped object.
    (sub as unknown as { writer: WritableStreamDefaultWriter<Uint8Array> }).writer = writer;
    (sub as unknown as { close: () => Promise<void> }).close = closeMe;

    this.subscribers.add(sub);

    // Replay buffered chunks first — fire-and-forget, the response below
    // returns immediately and the writer keeps pushing in the background.
    (async () => {
      for (const c of mem.buffer) {
        try { await writer.write(this.encoder.encode(this.sseFrame(c))); }
        catch { await closeMe(); return; }
      }
      if (mem.closed) {
        // Stream was already terminal when we attached — close after
        // replaying the buffer.
        await closeMe();
      }
    })().catch(() => { /* swallow — closeMe already handles cleanup */ });

    // Suppress unused-var error for controllerRef (kept for future
    // backpressure hook).
    void controllerRef;

    return new Response(readable, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-store",
        "Connection": "keep-alive",
        // Browsers default to 3s reconnect; bump so we don't hammer the
        // DO on transient drops.
        "X-Accel-Buffering": "no",
      },
    });
  }

  /// Mark the stream cancelled. Caller should also flip dispatches.status
  /// in D1 — that happens in the route handler, not the DO.
  private async handleCancel(mem: DispatchState): Promise<Response> {
    if (mem.closed) {
      return new Response(JSON.stringify({ already_closed: true }), {
        headers: { "Content-Type": "application/json" },
      });
    }
    const chunk: DispatchChunk = {
      seq: mem.next_seq++,
      kind: "status",
      data: "cancelled",
      ts: Date.now(),
    };
    mem.buffer.push(chunk);
    mem.closed = true;
    await this.persist();
    await this.state.storage.setAlarm(Date.now() + EVICTION_MS);
    this.fanOut(chunk);
    for (const sub of this.subscribers) this.closeSubscriber(sub);
    this.subscribers.clear();
    return new Response(JSON.stringify({ cancelled: true }), {
      headers: { "Content-Type": "application/json" },
    });
  }

  /// Push a chunk to every live subscriber. Failures (e.g. client
  /// disconnected) drop the subscriber from the set.
  private fanOut(chunk: DispatchChunk): void {
    const frame = this.encoder.encode(this.sseFrame(chunk));
    for (const sub of [...this.subscribers]) {
      const writer = (sub as unknown as { writer?: WritableStreamDefaultWriter<Uint8Array> }).writer;
      if (!writer) continue;
      writer.write(frame).catch(() => this.closeSubscriber(sub));
    }
  }

  /// Best-effort close + remove a subscriber.
  private closeSubscriber(sub: Subscriber): void {
    const closer = (sub as unknown as { close?: () => Promise<void> }).close;
    if (closer) {
      closer().catch(() => { /* ignore */ });
    }
    this.subscribers.delete(sub);
  }

  /// Format a chunk as an SSE frame. We use the event-type field so
  /// clients can switch on `event.type` cleanly in their handler.
  private sseFrame(c: DispatchChunk): string {
    // Escape any embedded newlines per SSE spec — each `data:` line is
    // a single newline-terminated record; multi-line payloads need
    // multiple data: lines.
    const dataLines = c.data.split("\n").map(l => `data: ${l}`).join("\n");
    return `id: ${c.seq}\nevent: ${c.kind}\n${dataLines}\n\n`;
  }

  /// Cloudflare invokes this when the eviction alarm fires.
  async alarm(): Promise<void> {
    // Purge our stored state + any leftover subscribers. The DO instance
    // itself will be torn down by the runtime once there are no in-flight
    // requests.
    for (const sub of this.subscribers) this.closeSubscriber(sub);
    this.subscribers.clear();
    await this.state.storage.deleteAll();
    this.mem = null;
  }
}
