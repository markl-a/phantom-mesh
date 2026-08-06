// Placeholder for F202 — Dispatch composer + live stream view.
// Will consume POST /api/me/dispatch/start + the SSE stream
// GET /api/me/dispatch/stream/:job_id (per F205 spec).
// TODO F205: wire apiPost + EventSource via api.ts.
export default function DispatchPage() {
  return (
    <section className="space-y-3">
      <h1 className="text-xl text-spectyn-primary">Dispatch</h1>
      <p className="text-sm text-spectyn-muted">
        Dispatch composer + live stream lands in F202.
      </p>
      <div
        data-testid="dispatch-placeholder"
        className="rounded border border-dashed border-spectyn-border p-6 text-spectyn-muted"
      >
        Placeholder shell — F202 fills this with the composer + stream.
      </div>
    </section>
  );
}
