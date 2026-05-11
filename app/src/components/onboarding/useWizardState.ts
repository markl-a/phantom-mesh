import { useState, useCallback, useEffect } from 'react';
import {
  OnboardingData, PersistedWizardState, UserIdentity, WizardStep,
  WIZARD_STORAGE_KEY, ONBOARDED_KEY,
} from './types';

const INITIAL_DATA: OnboardingData = {
  hardwareScan: null,
  identity: null,
  vaultPin: '',
  discoveredProviders: [],
  manualProviders: [],
  clusterEnabled: false,
  clusterNodes: [],
  telegramToken: '',
  qrPayload: null,
  ollamaEndpoint: 'http://localhost:11434',
  ollamaEnabled: false,
};

function loadPersistedState(): { step: WizardStep; data: Partial<OnboardingData> } {
  try {
    const raw = localStorage.getItem(WIZARD_STORAGE_KEY);
    if (!raw) return { step: 0, data: {} };
    const saved: PersistedWizardState = JSON.parse(raw);
    // Crash recovery: reset to step 1 (re-enter PIN) but keep identity if already authed
    const step = saved.currentStep >= 2 ? 1 as WizardStep : saved.currentStep as WizardStep;

    // Reconstruct minimal identity from persisted email/provider (skip OAuth re-auth)
    let identity: UserIdentity | null = null;
    if (saved.identityEmail && saved.identityProvider) {
      identity = {
        provider: saved.identityProvider as 'google' | 'apple',
        sub: '',  // sub not persisted — will be refreshed on next OAuth if needed
        email: saved.identityEmail,
        display_name: saved.identityEmail,
        avatar_url: null,
        id_token: null, // not persisted — will be refreshed on next OAuth
      };
    }

    return {
      step,
      data: {
        identity,
        ollamaEnabled: saved.ollamaEnabled,
        ollamaEndpoint: saved.ollamaEndpoint,
        clusterEnabled: saved.clusterEnabled,
        clusterNodes: saved.clusterNodes,
      },
    };
  } catch {
    return { step: 0, data: {} };
  }
}

export function useWizardState() {
  const persisted = loadPersistedState();

  const [currentStep, setCurrentStep] = useState<WizardStep>(persisted.step);
  const [data, setData] = useState<OnboardingData>({
    ...INITIAL_DATA,
    ...persisted.data,
  });

  // Persist non-sensitive state on step change
  useEffect(() => {
    const state: PersistedWizardState = {
      currentStep,
      ollamaEnabled: data.ollamaEnabled,
      ollamaEndpoint: data.ollamaEndpoint,
      providerNames: data.manualProviders.map(p => p.name),
      discoveredProviderNames: data.discoveredProviders
        .filter(p => p.enabled)
        .map(p => p.name),
      clusterEnabled: data.clusterEnabled,
      clusterNodes: data.clusterNodes,
      telegramConfigured: !!data.telegramToken,
      identityEmail: data.identity?.email,
      identityProvider: data.identity?.provider,
    };
    localStorage.setItem(WIZARD_STORAGE_KEY, JSON.stringify(state));
  }, [currentStep, data]);

  const goNext = useCallback(() => {
    setCurrentStep(s => Math.min(s + 1, 5) as WizardStep);
  }, []);

  const goBack = useCallback(() => {
    setCurrentStep(s => Math.max(s - 1, 0) as WizardStep);
  }, []);

  const goTo = useCallback((step: WizardStep) => {
    setCurrentStep(step);
  }, []);

  const updateData = useCallback((partial: Partial<OnboardingData>) => {
    setData(prev => ({ ...prev, ...partial }));
  }, []);

  const completeWizard = useCallback(() => {
    localStorage.setItem(ONBOARDED_KEY, 'true');
    localStorage.removeItem(WIZARD_STORAGE_KEY);
  }, []);

  return { currentStep, data, goNext, goBack, goTo, updateData, completeWizard };
}
