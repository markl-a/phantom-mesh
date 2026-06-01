// Placeholder for F202 — Dispatch composer + live stream view.
// Will consume POST /api/me/dispatch/start + the SSE stream
// GET /api/me/dispatch/stream/:job_id (per F205 spec).
// TODO F205: wire apiPost + EventSource via api.ts.
export default function DispatchPage() {
  return (
    <section className="space-y-3">
      <h1 className="text-xl text-phantom-primary">Dispatch</h1>
      <p className="text-sm text-phantom-muted">
        Dispatch composer + live stream lands in F202.
      </p>
      <div
        data-testid="dispatch-placeholder"
        className="rounded border border-dashed border-phantom-border p-6 text-phantom-muted"
      >
        Placeholder shell — F202 fills this with the composer + stream.
      </div>
    </section>
  );
}
