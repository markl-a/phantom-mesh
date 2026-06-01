import { useEffect, useState, type ReactNode } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  KeyRound,
  Network,
  Plus,
  Send,
  Zap,
} from "lucide-react";
import type { OnboardingWizardProps, ClusterChoice } from "./types";

type Step = 1 | 2 | 3 | 4;

const clusterOptions: Array<{
  value: ClusterChoice;
  label: string;
  note: string;
}> = [
  {
    value: "join_existing",
    label: "加入既有叢集",
    note: "自動發現區網內的節點 (mDNS)",
  },
  {
    value: "create_new",
    label: "建立新叢集",
    note: "成為第一個節點",
  },
  {
    value: "single_machine",
    label: "之後再說",
    note: "先以單機模式執行",
  },
];

function OnboardingWizard({
  initialStep,
  onClose,
  onComplete,
  onAddProvider,
  onUseDemoRelay,
  onSendFirstMessage,
}: OnboardingWizardProps) {
  const [step, setStep] = useState<Step>((initialStep ?? 1) as Step);
  const [cluster, setCluster] = useState<ClusterChoice | null>(null);
  const [providerReady, setProviderReady] = useState(false);
  const [chatText, setChatText] = useState("say hello");
  const [firstMessageSent, setFirstMessageSent] = useState(false);

  // useState only seeds `step` on mount; sync post-mount `initialStep`
  // prop changes (e.g. caller resumes onboarding at a later step).
  useEffect(() => {
    if (initialStep) setStep(initialStep as Step);
  }, [initialStep]);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose?.();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  // SPEC-41 §10 flow: step 4 ("首次對話") completes only after the user has
  // actually sent their first message, not just by reaching the step.
  const canContinue =
    step === 1 ||
    (step === 2 && cluster !== null) ||
    (step === 3 && providerReady) ||
    (step === 4 && firstMessageSent);

  const goBack = () => setStep((s) => Math.max(1, s - 1) as Step);

  const goForward = () => {
    if (step === 4) {
      onComplete?.({
        cluster: cluster ?? "single_machine",
        providerConfigured: providerReady,
      });
      onClose?.();
      return;
    }

    setStep((s) => Math.min(4, s + 1) as Step);
  };

  return (
    <div
      data-testid="onboarding-wizard"
      className="flex min-h-screen items-center justify-center bg-phantom-bg p-6 text-phantom-text"
    >
      <section className="w-[560px] max-w-full rounded-xl border border-phantom-border bg-phantom-card shadow-2xl">
        <div className="border-b border-phantom-border px-8 py-6">
          <div className="mb-6 flex items-center justify-center gap-3">
            {[1, 2, 3, 4].map((dot) => (
              <div
                key={dot}
                className={[
                  "h-2.5 w-2.5 rounded-full transition-colors",
                  dot === step
                    ? "bg-phantom-primary"
                    : dot < step
                      ? "bg-phantom-primary/50"
                      : "bg-phantom-border",
                ].join(" ")}
              />
            ))}
          </div>

          <div className="text-center text-sm text-phantom-muted">
            {step} / 4
          </div>
        </div>

        <div className="min-h-[360px] px-8 py-7">{renderStep()}</div>

        <footer className="flex items-center justify-between border-t border-phantom-border px-8 py-5">
          <button
            type="button"
            onClick={goBack}
            disabled={step === 1}
            className={[
              "inline-flex items-center gap-2 rounded-lg border border-phantom-border px-4 py-2 text-sm transition",
              step === 1
                ? "invisible cursor-default text-phantom-muted"
                : "text-phantom-text hover:border-phantom-primary hover:text-phantom-primary",
            ].join(" ")}
          >
            <ArrowLeft className="h-4 w-4" />
            返回
          </button>

          <button
            type="button"
            onClick={goForward}
            disabled={!canContinue}
            className={[
              "inline-flex items-center gap-2 rounded-lg px-5 py-2 text-sm font-medium transition",
              canContinue
                ? "bg-phantom-primary text-white hover:bg-phantom-primary/90"
                : "cursor-not-allowed bg-phantom-border text-phantom-muted",
            ].join(" ")}
          >
            {step === 4 ? "完成" : "繼續"}
            {step === 4 ? (
              <Check className="h-4 w-4" />
            ) : (
              <ArrowRight className="h-4 w-4" />
            )}
          </button>
        </footer>
      </section>
    </div>
  );

  function renderStep() {
    if (step === 1) {
      return (
        <div className="space-y-5">
          <StepHeader
            icon={<KeyRound className="h-7 w-7" />}
            title="步驟 1 — 身分金鑰"
          />
          <p className="leading-7 text-phantom-muted">
            Phantom Mesh 用本機產生的 32-byte ed25519
            金鑰當你的身分；金鑰只存在這台 Mac 的 Keychain，永不上雲。
          </p>
        </div>
      );
    }

    if (step === 2) {
      return (
        <div className="space-y-5">
          <StepHeader
            icon={<Network className="h-7 w-7" />}
            title="步驟 2 — 叢集"
          />
          <div className="space-y-3">
            {clusterOptions.map((option) => {
              const selected = cluster === option.value;

              return (
                <button
                  key={option.value}
                  type="button"
                  onClick={() => setCluster(option.value)}
                  className={[
                    "w-full rounded-lg border p-4 text-left transition",
                    selected
                      ? "border-phantom-primary bg-phantom-primary/10"
                      : "border-phantom-border bg-phantom-bg hover:border-phantom-primary",
                  ].join(" ")}
                >
                  <div className="flex items-start gap-3">
                    <span
                      className={[
                        "mt-1 h-3.5 w-3.5 rounded-full border",
                        selected
                          ? "border-phantom-primary bg-phantom-primary"
                          : "border-phantom-muted",
                      ].join(" ")}
                    />
                    <span>
                      <span className="block font-medium text-phantom-text">
                        {option.label}
                      </span>
                      <span className="mt-1 block text-sm text-phantom-muted">
                        {option.note}
                      </span>
                    </span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      );
    }

    if (step === 3) {
      return (
        <div className="space-y-5">
          <StepHeader
            icon={<Plus className="h-7 w-7" />}
            title="步驟 3 — 供應商"
          />
          <p className="leading-7 text-phantom-muted">
            至少設定一個模型供應商（demo-relay 不算）才能繼續。
          </p>
          <div className="flex flex-col gap-3">
            <button
              type="button"
              onClick={() => {
                onAddProvider?.();
                setProviderReady(true);
              }}
              className="inline-flex items-center justify-center gap-2 rounded-lg bg-phantom-primary px-4 py-3 text-sm font-medium text-white hover:bg-phantom-primary/90"
            >
              <Plus className="h-4 w-4" />
              新增供應商
            </button>
            <button
              type="button"
              onClick={() => onUseDemoRelay?.()}
              className="inline-flex items-center justify-center gap-2 rounded-lg border border-phantom-border px-4 py-3 text-sm text-phantom-text hover:border-phantom-primary hover:text-phantom-primary"
            >
              <Zap className="h-4 w-4" />
              使用 demo-relay（30 秒免設定）
            </button>
          </div>
          {providerReady ? (
            <div className="inline-flex items-center gap-2 rounded-lg border border-phantom-primary bg-phantom-primary/10 px-3 py-2 text-sm text-phantom-primary">
              <Check className="h-4 w-4" />
              已設定供應商 ✓
            </div>
          ) : null}
        </div>
      );
    }

    return (
      <div className="space-y-5">
        <StepHeader
          icon={<Send className="h-7 w-7" />}
          title="步驟 4 — 首次對話"
        />
        <textarea
          value={chatText}
          onChange={(event) => setChatText(event.target.value)}
          className="min-h-32 w-full resize-none rounded-lg border border-phantom-border bg-phantom-bg p-4 text-sm text-phantom-text outline-none focus:border-phantom-primary"
        />
        <button
          type="button"
          onClick={() => {
            onSendFirstMessage?.(chatText);
            setFirstMessageSent(true);
          }}
          className="inline-flex items-center gap-2 rounded-lg bg-phantom-primary px-4 py-2 text-sm font-medium text-white hover:bg-phantom-primary/90"
        >
          <Send className="h-4 w-4" />
          傳送
        </button>
        <p className="text-sm text-phantom-muted">
          {firstMessageSent
            ? "已傳送 ✓ — 回覆會串流顯示於主對話視窗"
            : "傳送後即可完成設定；回覆會串流顯示於主對話視窗"}
        </p>
      </div>
    );
  }
}

function StepHeader({
  icon,
  title,
}: {
  icon: ReactNode;
  title: string;
}) {
  return (
    <div className="flex items-center gap-3">
      <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-phantom-primary/10 text-phantom-primary">
        {icon}
      </div>
      <h1 className="text-xl font-semibold text-phantom-text">{title}</h1>
    </div>
  );
}

export default OnboardingWizard;
