// J1 onboarding wizard host (SPEC-34 Screen 1 / Flow 1 / SPEC-28). Renders the
// active step, shows a "第 X 步，共 N 步" indicator, and drives the SPEC-28 FSM
// via advanceOnboarding (BigInt-safe — see lib/onboardingFsm.ts toJsonSafe).
import { useState } from "react";
import {
  WIZARD_STEPS, nextStep, prevStep, markOnboarded, type WizardStep,
} from "../../lib/onboardingWizard";
import { advanceOnboarding } from "../../lib/onboardingFsm";
import WelcomeStep from "./onboarding/WelcomeStep";
import PermissionsStep from "./onboarding/PermissionsStep";
import PaletteStep from "./onboarding/PaletteStep";
import MeshStep from "./onboarding/MeshStep";
import DoneStep from "./onboarding/DoneStep";

export default function MobileOnboardingWizard({
  onDone,
  onImport,
}: {
  onDone: () => void;
  onImport: () => void;
}) {
  const [step, setStep] = useState<WizardStep>("welcome");
  const idx = WIZARD_STEPS.indexOf(step);

  // advanceOnboarding carries the SPEC-28 FSM forward; it is BigInt-safe and
  // soft-fails (client fallback) when the backend isn't wired, so the wizard
  // never blocks. We ignore its result and rely on the local step machine.
  const advance = () => {
    void advanceOnboarding().catch(() => {});
    setStep((s) => nextStep(s));
  };

  const finish = () => {
    markOnboarded();
    onDone();
  };

  return (
    <div className="flex flex-col h-[100dvh] bg-phantom-bg">
      <div className="px-6 pt-6 text-center text-xs text-phantom-muted" aria-live="polite">
        第 {idx + 1} 步，共 {WIZARD_STEPS.length} 步
      </div>
      <div className="flex-1 flex flex-col justify-center">
        {step === "welcome" && <WelcomeStep onNext={advance} onImport={onImport} />}
        {step === "permissions" && <PermissionsStep onNext={advance} />}
        {step === "palette" && <PaletteStep onNext={advance} />}
        {step === "mesh" && <MeshStep onNext={advance} onConnect={onImport} />}
        {step === "done" && (
          <DoneStep summary={"權限與習慣已設定\n可隨時在 設定 調整"} onFinish={finish} />
        )}
      </div>
      {idx > 0 && step !== "done" && (
        <button
          onClick={() => setStep((s) => prevStep(s))}
          className="px-6 pb-6 text-xs text-phantom-muted self-start"
        >
          ← 上一步
        </button>
      )}
    </div>
  );
}
