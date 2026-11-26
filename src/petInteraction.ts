export const DEFAULT_IDLE_DELAY_SECONDS = 15;
export const MIN_IDLE_DELAY_SECONDS = 5;
export const MAX_IDLE_DELAY_SECONDS = 600;

export function idleDelaySeconds(value: number | undefined, embedded = false): number {
  if (value === undefined || !Number.isFinite(value)) return embedded ? 18 : DEFAULT_IDLE_DELAY_SECONDS;
  return Math.min(MAX_IDLE_DELAY_SECONDS, Math.max(MIN_IDLE_DELAY_SECONDS, Math.round(value)));
}

export function isHeadTap(durationMs: number, moved: boolean): boolean {
  return !moved && durationMs >= 0 && durationMs <= 350;
}

const STRETCH_MS = 1800;

const MIN_SLEEP_TAIL_DELAY_MS = 180_000;
const SLEEP_TAIL_MS = 1000;

export class SleepTransition {
  private stretchSelected: boolean | undefined;
  private sleepTailAt: number | undefined;

  constructor(private readonly random = Math.random) {}

  update(idleMs: number, delayMs: number, animate: boolean, alreadyAsleep = false): {
    asleep: boolean;
    stretch: number;
    sleepTail: number;
  } {
    if (idleMs < delayMs && !alreadyAsleep) {
      this.stretchSelected = undefined;
      this.sleepTailAt = undefined;
      return { asleep: false, stretch: 0, sleepTail: 0 };
    }
    if (!animate) return { asleep: true, stretch: 0, sleepTail: 0 };

    if (!alreadyAsleep) {
      this.stretchSelected ??= this.random() < 0.55;
      const progress = (idleMs - delayMs) / STRETCH_MS;
      if (this.stretchSelected && progress < 1) {
        return { asleep: false, stretch: Math.sin(progress * Math.PI), sleepTail: 0 };
      }
    }

    this.sleepTailAt ??= idleMs + MIN_SLEEP_TAIL_DELAY_MS * (1 + this.random());
    const phase = (idleMs - this.sleepTailAt) / SLEEP_TAIL_MS;
    const sleepTail = phase > 0 && phase < 1 ? Math.sin(phase * 2 * Math.PI) * Math.sin(phase * Math.PI) : 0;
    return { asleep: true, stretch: 0, sleepTail };
  }
}

const TAP_MS = 800;
const STROKE_GAP_MS = 450;
const ACCENT_MS = 750;
const ACCENTS = ['tail', 'rub', 'ear'] as const;
type Accent = typeof ACCENTS[number];

export class PetGesture {
  private tapAt = -Infinity;
  private tapDirection = 0;
  private strokeAt = -Infinity;
  private strokeStart = -Infinity;
  private strokeDirection = 0;
  private accentAt = -Infinity;
  private accented = false;
  private accent: Accent | undefined;

  constructor(private readonly random = Math.random) {}

  tap(now: number, direction: number): void {
    if (now - this.tapAt < TAP_MS) return;
    this.tapAt = now;
    this.tapDirection = direction;
  }

  stroke(now: number, direction: number): void {
    if (now - this.strokeAt > STROKE_GAP_MS) {
      this.strokeStart = now;
      this.accented = false;
    }
    this.strokeDirection = Math.max(-1, Math.min(1, direction));
    this.strokeAt = now;
    if (!this.accented && now - this.strokeStart >= 900) {
      const choices = ACCENTS.filter((kind) => kind !== this.accent);
      this.accent = choices[Math.floor(this.random() * choices.length)];
      this.accented = true;
      this.accentAt = now;
    }
  }

  pose(now: number): { tap: number; lean: number; tail: number; ear: number } {
    const tapPhase = (now - this.tapAt) / TAP_MS;
    const tap = tapPhase > 0 && tapPhase < 1 ? Math.sin(tapPhase * Math.PI) : 0;
    const stroke = Math.max(0, 1 - Math.max(0, now - this.strokeAt) / STROKE_GAP_MS);
    const accentPhase = (now - this.accentAt) / ACCENT_MS;
    const accent = accentPhase > 0 && accentPhase < 1 ? Math.sin(accentPhase * Math.PI) : 0;
    const strokeTail = this.accent === 'tail' && accent > 0 ? Math.sin(accentPhase * 4 * Math.PI) * accent : 0;
    return {
      tap,
      lean: this.tapDirection * tap + this.strokeDirection * stroke * (this.accent === 'rub' ? 1 + 0.5 * accent : 1),
      tail: strokeTail + (tap > 0 ? Math.sin(tapPhase * 2 * Math.PI) * tap * 0.65 : 0),
      ear: this.accent === 'ear' && accent > 0 ? Math.sin(accentPhase * 2 * Math.PI) * accent : 0,
    };
  }

  clear(): void {
    this.tapAt = this.strokeAt = this.accentAt = -Infinity;
  }
}
