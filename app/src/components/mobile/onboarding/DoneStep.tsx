// J1 Step 5 — Done (SPEC-34 Screen 1). Recap + 開始使用 → lands on chat.
export default function DoneStep({
  summary,
  onFinish,
}: {
  summary: string;
  onFinish: () => void;
}) {
  return (
    <div className="px-6 space-y-4 text-center">
      <h2 className="text-xl font-bold text-phantom-success">設定完成！</h2>
      <p className="text-sm text-phantom-text whitespace-pre-line">{summary}</p>
      <button
        onClick={onFinish}
        className="w-full bg-phantom-primary text-phantom-bg py-2.5 rounded-lg text-sm font-medium hover:brightness-110 transition"
      >
        開始使用
      </button>
    </div>
  );
}
