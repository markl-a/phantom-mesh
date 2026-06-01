// J1 Step 4 — Mesh join (optional, SPEC-34 Screen 1). onConnect is wired by the
// host to the existing token-import flow; 先略過 advances without joining.
export default function MeshStep({
  onNext,
  onConnect,
}: {
  onNext: () => void;
  onConnect: () => void;
}) {
  return (
    <div className="px-6 space-y-4 text-center">
      <h2 className="text-lg font-bold text-phantom-text">要連上你的裝置叢集嗎？</h2>
      <p className="text-xs text-phantom-muted">
        把手機加入你其他電腦的 mesh，可跨裝置同步與派送任務。
      </p>
      <button
        onClick={onConnect}
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
      >
        連接叢集
      </button>
      <button
        onClick={onNext}
        className="w-full text-sm text-phantom-muted border border-phantom-border rounded-lg py-2.5"
      >
        先略過
      </button>
    </div>
  );
}
