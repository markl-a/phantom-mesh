import { useEffect } from 'react';
import { useWizardState } from './useWizardState';
import { useHardwareScan } from './useHardwareScan';
import StepWelcome from './StepWelcome';
import StepSecurity from './StepSecurity';
import StepProviderDiscovery from './StepProviderDiscovery';
import StepProviderManual from './StepProviderManual';
import StepNetwork from './StepNetwork';
import StepComplete from './StepComplete';

interface Props {
  onComplete: () => void;
}

export default function OnboardingWizard({ onComplete }: Props) {
  const { currentStep, data, goNext, goBack, goTo, updateData, completeWizard } = useWizardState();
  const scan = useHardwareScan(true);

  // Pass scan result to data when ready
  useEffect(() => {
    if (scan.status === 'done' && scan.result && !data.hardwareScan) {
      updateData({ hardwareScan: scan.result });
    }
  }, [scan.status, scan.result, data.hardwareScan, updateData]);

  const steps = [
    <StepWelcome key="welcome" scan={scan} onNext={goNext} />,
    <StepSecurity key="security" data={data} updateData={updateData} onNext={goNext} onBack={goBack} />,
    <StepProviderDiscovery key="discovery" data={data} updateData={updateData} goNext={goNext} goBack={goBack} goTo={goTo} />,
    <StepProviderManual key="manual" data={data} updateData={updateData} goNext={goNext} goBack={goBack} goTo={goTo} />,
    <StepNetwork key="network" data={data} updateData={updateData} onNext={goNext} onBack={goBack} />,
    <StepComplete key="complete" data={data} completeWizard={completeWizard} onComplete={onComplete} onBack={goBack} />,
  ];

  return (
    <div className="h-screen bg-phantom-bg flex flex-col items-center overflow-y-auto p-8">
      {/* Progress dots */}
      <div className="flex gap-2 mb-8 mt-4 flex-shrink-0">
        {[0, 1, 2, 3, 4, 5].map(i => (
          <div
            key={i}
            className={`w-2.5 h-2.5 rounded-full transition-colors ${
              i === currentStep
                ? 'bg-phantom-primary'
                : i < currentStep
                  ? 'bg-phantom-primary/50'
                  : 'bg-phantom-border'
            }`}
          />
        ))}
      </div>
      {/* Current step */}
      <div className="w-full max-w-xl flex-shrink-0">
        {steps[currentStep]}
      </div>
    </div>
  );
}
