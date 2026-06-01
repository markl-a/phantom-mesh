// Placeholder for F204 — User preferences + session management.
// Will consume:
//   GET/PUT /api/me/preferences
//   GET/PUT /api/me/peer-capabilities
//   DELETE  /api/me/sessions/all-others
// (per F205 spec.)
// TODO F205: wire apiGet/apiPut/apiDelete.
export default function SettingsPage() {
  return (
    <section className="space-y-3">
      <h1 className="text-xl text-phantom-primary">Settings</h1>
      <p className="text-sm text-phantom-muted">
        Preferences, capabilities, session management land in F204.
      </p>
      <div
        data-testid="settings-placeholder"
        className="rounded border border-dashed border-phantom-border p-6 text-phantom-muted"
      >
        Placeholder shell — F204 fills this with the settings panes.
      </div>
    </section>
  );
}
