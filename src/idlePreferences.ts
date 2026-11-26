import { idleDelaySeconds } from './petInteraction';

interface IdlePreferences {
  idleDelaySeconds?: number;
  idleAnimationsEnabled?: boolean;
}

const delayInput = document.querySelector<HTMLInputElement>('#idle-delay-seconds')!;
const animationInput = document.querySelector<HTMLInputElement>('#idle-animations-enabled')!;

export function syncIdleControls(settings: IdlePreferences): void {
  if (document.activeElement !== delayInput) {
    delayInput.value = String(idleDelaySeconds(settings.idleDelaySeconds));
  }
  animationInput.checked = settings.idleAnimationsEnabled !== false;
}

export function bindIdleControls(
  getSettings: () => IdlePreferences,
  update: (patch: IdlePreferences) => Promise<void>,
): void {
  delayInput.addEventListener('change', () => {
    const current = idleDelaySeconds(getSettings().idleDelaySeconds);
    const next = Number.isFinite(delayInput.valueAsNumber)
      ? idleDelaySeconds(delayInput.valueAsNumber)
      : current;
    delayInput.value = String(next);
    if (next !== current) void update({ idleDelaySeconds: next });
  });
  animationInput.addEventListener('change', () => {
    void update({ idleAnimationsEnabled: animationInput.checked });
  });
}
