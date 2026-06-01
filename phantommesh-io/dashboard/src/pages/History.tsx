// Placeholder for F203 — Dispatch history list + detail drawer.
// Will consume GET /api/me/dispatches (paginated) + GET /api/me/dispatches/:job_id
// (per F205 spec).
// TODO F205: wire apiGet with pagination.
export default function HistoryPage() {
  return (
    <section className="space-y-3">
      <h1 className="text-xl text-phantom-primary">History</h1>
      <p className="text-sm text-phantom-muted">
        Past dispatch list + detail drawer lands in F203.
      </p>
      <div
        data-testid="history-placeholder"
        className="rounded border border-dashed border-phantom-border p-6 text-phantom-muted"
      >
        Placeholder shell — F203 fills this with the paginated list.
      </div>
    </section>
  );
}
