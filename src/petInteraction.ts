export const DEFAULT_IDLE_DELAY_SECONDS = 15;
export const MIN_IDLE_DELAY_SECONDS = 5;
export const MAX_IDLE_DELAY_SECONDS = 600;

export function idleDelaySeconds(value: number | undefined, embedded = false): number {
  if (value === undefined || !Number.isFinite(value)) return embedded ? 18 : DEFAULT_IDLE_DELAY_SECONDS;
  return Math.min(MAX_IDLE_DELAY_SECONDS, Math.max(MIN_IDLE_DELAY_SECONDS, Math.round(value)));
}

export function isPetTap(durationMs: number, moved: boolean): boolean {
  return !moved && durationMs >= 0 && durationMs <= 350;
}

export type PawSide = 'left' | 'right';

type Point = { x: number; y: number };

export function pawSideAt(cursor: Point, left: Point, right: Point, radius: number): PawSide | null {
  const leftDistance = Math.hypot(cursor.x - left.x, cursor.y - left.y);
  const rightDistance = Math.hypot(cursor.x - right.x, cursor.y - right.y);
  if (Math.min(leftDistance, rightDistance) > radius) return null;
  return leftDistance <= rightDistance ? 'left' : 'right';
}

export function isWakeBurst(keyTimes: readonly number[], window: number): boolean {
  const first = keyTimes[keyTimes.length - 4];
  return first !== undefined && keyTimes[keyTimes.length - 1] - first <= window;
}

export function gentleWakeAmount(elapsedMs: number): number {
  const phase = elapsedMs / 480;
  return phase > 0 && phase < 1 ? Math.sin(Math.PI * phase) ** 2 : 0;
}

const HIGH_FIVE_MS = 800;

function smoothUnit(value: number): number {
  const u = Math.max(0, Math.min(1, value));
  return u * u * (3 - 2 * u);
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

const TAIL_FOLLOW_THROUGH_MS = 150;

export interface TailSwing {
  elapsedMs: number;
  durationMs: number;
  strength: number;
  direction: number;
}

interface TailPoint {
  x: number;
  y: number;
  z?: number;
}

export function idleTailSwing(elapsedMs: number, totalMs: number, direction: number): TailSwing {
  return { elapsedMs, durationMs: totalMs - TAIL_FOLLOW_THROUGH_MS, strength: 0.42, direction };
}

/** Rotate the segments of a tail curve, keeping their lengths and the attachment point fixed. */
export function bendTail(points: TailPoint[], swing: TailSwing, weight = 1): void {
  if (points.length < 2 || swing.elapsedMs <= 0 || swing.elapsedMs >= swing.durationMs + TAIL_FOLLOW_THROUGH_MS) {
    return;
  }
  const spatial = points[0].z !== undefined;
  let restX = points[0].x;
  let restY = points[0].y;
  let restZ = points[0].z ?? 0;
  for (let i = 1; i < points.length; i++) {
    const point = points[i];
    const previous = points[i - 1];
    let dx = point.x - restX;
    const dy = point.y - restY;
    let dz = (point.z ?? 0) - restZ;
    restX = point.x;
    restY = point.y;
    restZ = point.z ?? 0;

    const along = (i - 1) / Math.max(1, points.length - 2);
    const phase = (swing.elapsedMs - along * TAIL_FOLLOW_THROUGH_MS) / swing.durationMs;
    const fade = Math.max(0, Math.min(1, phase / 0.22, (1 - phase) / 0.24));
    const eased = fade * fade * (3 - 2 * fade);
    // The hip moves first; the outer segments reverse later and finish the return more softly.
    const angle = -Math.sin(phase * 2 * Math.PI) * eased * (1 - 0.14 * phase) *
      (0.72 + 0.28 * along) * swing.strength * swing.direction * weight;
    let lift = angle;
    if (spatial) {
      const yaw = angle * 0.85;
      const sweptX = dx * Math.cos(yaw) - dz * Math.sin(yaw);
      dz = dx * Math.sin(yaw) + dz * Math.cos(yaw);
      dx = sweptX;
      // Pitch with the sweep so the forward/back motion is visible from the front camera.
      lift = -angle * 0.34 + Math.abs(angle) * 0.09;
    }
    point.x = previous.x + dx * Math.cos(lift) - dy * Math.sin(lift);
    point.y = previous.y + dx * Math.sin(lift) + dy * Math.cos(lift);
    if (spatial) point.z = previous.z! + dz;
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
  private tailAt = -Infinity;
  private tailSweep: Omit<TailSwing, 'elapsedMs'> | null = null;
  private accent: Accent | undefined;
  private highFiveAt = -Infinity;
  private highFiveSide: PawSide = 'left';

  constructor(private readonly random = Math.random) {}

  tap(now: number, direction: number, delayMs = 0): void {
    if (now - this.tapAt < TAP_MS) return;
    this.tapAt = now + delayMs;
    this.tapDirection = direction;
    this.startTailSwing(this.tapAt, 1150, 0.70, direction);
  }

  highFive(now: number, side: PawSide): void {
    if (now - this.highFiveAt < HIGH_FIVE_MS) return;
    this.highFiveAt = now;
    this.highFiveSide = side;
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
      if (this.accent === 'tail') this.startTailSwing(now, 1550, 0.95, direction);
    }
  }

  private startTailSwing(now: number, durationMs: number, strength: number, direction: number): void {
    if (this.tailSweep && now - this.tailAt < this.tailSweep.durationMs + TAIL_FOLLOW_THROUGH_MS) return;
    this.tailAt = now;
    this.tailSweep = { durationMs, strength, direction: Math.sign(direction) || 1 };
  }

  pose(now: number): {
    tap: number; lean: number; tail: TailSwing | null; ear: number;
    highFive: { side: PawSide; amount: number } | null;
  } {
    const tapPhase = (now - this.tapAt) / TAP_MS;
    const tap = tapPhase > 0 && tapPhase < 1 ? Math.sin(tapPhase * Math.PI) : 0;
    const stroke = Math.max(0, 1 - Math.max(0, now - this.strokeAt) / STROKE_GAP_MS);
    const accentPhase = (now - this.accentAt) / ACCENT_MS;
    const accent = accentPhase > 0 && accentPhase < 1 ? Math.sin(accentPhase * Math.PI) : 0;
    const elapsedMs = now - this.tailAt;
    const tail = this.tailSweep && elapsedMs >= 0 && elapsedMs < this.tailSweep.durationMs + TAIL_FOLLOW_THROUGH_MS
      ? { ...this.tailSweep, elapsedMs }
      : null;
    const fiveAge = now - this.highFiveAt;
    const fiveAmount = smoothUnit(fiveAge / 240) * smoothUnit((HIGH_FIVE_MS - fiveAge) / 420);
    return {
      highFive: fiveAmount > 0 ? { side: this.highFiveSide, amount: fiveAmount } : null,
      tap,
      lean: this.tapDirection * tap + this.strokeDirection * stroke * (this.accent === 'rub' ? 1 + 0.5 * accent : 1),
      tail,
      ear: this.accent === 'ear' && accent > 0 ? Math.sin(accentPhase * 2 * Math.PI) * accent : 0,
    };
  }

  clear(): void {
    this.tapAt = this.strokeAt = this.accentAt = this.tailAt = this.highFiveAt = -Infinity;
    this.tailSweep = null;
  }
}
