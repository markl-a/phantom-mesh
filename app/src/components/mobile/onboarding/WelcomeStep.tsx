// J1 Step 1 — Welcome (SPEC-34 Screen 1). Folds language pick into the device
// locale default; "我有匯入碼" routes to the existing token-import path.
export default function WelcomeStep({
  onNext,
  onImport,
}: {
  onNext: () => void;
  onImport: () => void;
}) {
  return (
    <div className="flex flex-col items-center text-center gap-4 px-6">
      <h2 className="text-xl font-bold text-phantom-text">歡迎使用 Phantom Mesh</h2>
      <p className="text-sm text-phantom-muted">你的私人 AI 助理，資料留在自己裝置。</p>
      <button
        onClick={onNext}
        aria-label="開始設定，進入下一步"
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
      >
        開始設定
      </button>
      <button onClick={onImport} className="text-xs text-phantom-muted hover:underline">
        我有匯入碼
      </button>
    </div>
  );
}
