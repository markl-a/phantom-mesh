// Placeholder for F201 — Cluster overview screen.
// Will consume GET /api/me/cluster-peers/:peer/caps (per F205 spec).
// TODO F205: replace mockPeers below with apiGet("/api/me/cluster-peers").
export default function ClusterPage() {
  return (
    <section className="space-y-3">
      <h1 className="text-xl text-spectyn-primary">Cluster</h1>
      <p className="text-sm text-spectyn-muted">
        Peer + capability overview lands in F201.
      </p>
      <div
        data-testid="cluster-placeholder"
        className="rounded border border-dashed border-spectyn-border p-6 text-spectyn-muted"
      >
        Placeholder shell — F201 fills this with the peer table.
      </div>
    </section>
  );
}
