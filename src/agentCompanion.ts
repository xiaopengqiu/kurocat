import { catAudio } from './audio';

export type AgentStatusKind = 'working' | 'thinking' | 'waiting' | 'done' | 'error' | 'idle';
export type AgentActionKind =
  | 'happy'
  | 'pet'
  | 'angry'
  | 'curious'
  | 'sleep'
  | 'wake'
  | 'tail_pat';

export interface AgentCompanionEvent {
  kind: 'status' | 'action' | 'bubble';
  status?: AgentStatusKind | string;
  action?: AgentActionKind | string;
  source?: string;
  text?: string;
  textIsDefault?: boolean;
  errorKind?: 'tool' | 'task';
  tool?: string;
  autoType?: boolean;
  sound?: boolean;
  ttlMs?: number;
  ts?: number;
}

export interface AgentCompanionTarget {
  onKeyDown(vk: number, repeat?: boolean): void;
  onKeyUp(vk: number): void;
  triggerCuriousDemo(duration?: number): void;
  triggerCrossDemo(): void;
  triggerSleepDemo(): void;
  triggerHappyDemo?(duration?: number): void;
  triggerWakeDemo?(): void;
  triggerTailPatDemo?(): void;
}

// Natural coding keystroke VKs (home-row letters, symbols, space, enter, backspace)
const CODING_VKS: readonly number[] = [
  0x41, 0x43, 0x44, 0x45, 0x46, 0x47, 0x49, 0x4c, 0x4d, 0x4e,
  0x4f, 0x50, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x20,
  0x20, 0x0d, 0x08, 0xbc, 0xbe,
];

const CAPTIONS = {
  tool: ['这步没跑通', '执行遇到点问题', '这条指令卡住了'],
  task: ['这轮卡住了', '任务在这里中断', '这次没跑完'],
  recovery: ['继续处理', '接着来', '重新开始处理'],
} as const;
const ERROR_DEDUP_MS = 2_500;
const RECOVERY_WINDOW_MS = 120_000;
const RECOVERY_BUBBLE_MS = 3_200;

type CaptionKind = keyof typeof CAPTIONS;
type RecentError = { kind: 'tool' | 'task'; at: number; tool?: string; text?: string };

export interface AgentCompanionOptions {
  bubbleEl: HTMLDivElement | null;
  getTarget: () => AgentCompanionTarget | null;
  is3DTarget: () => boolean;
  allowSound: () => boolean;
}

export class AgentCompanionController {
  private readonly bubbleEl: HTMLDivElement | null;
  private readonly getTarget: () => AgentCompanionTarget | null;
  private readonly is3DTarget: () => boolean;
  private readonly allowSound: () => boolean;

  private activeStatus: AgentStatusKind = 'idle';
  private privacyPaused = false;
  private autoTyping = false;
  private realUserInputUntil = 0;
  private typeTimer: number | null = null;
  private releaseTimers = new Set<number>();
  private heldVirtualVks = new Set<number>();
  private ttlTimer: number | null = null;
  private burstRemaining = 0;
  private lastSoundAt = 0;
  private readonly recentErrors = new Map<string, RecentError>();
  private readonly lastCaption: Partial<Record<CaptionKind, string>> = {};
  private recoveryBubble: { source: string; text: string; until: number } | null = null;

  constructor(opts: AgentCompanionOptions) {
    this.bubbleEl = opts.bubbleEl;
    this.getTarget = opts.getTarget;
    this.is3DTarget = opts.is3DTarget;
    this.allowSound = opts.allowSound;
  }

  public setPrivacyMode(active: boolean): void {
    this.privacyPaused = Boolean(active);
    if (this.privacyPaused) {
      this.recoveryBubble = null;
      this.releaseAllVirtualKeys();
      this.hideBubble();
    }
  }

  /** Pause synthetic coding keystrokes briefly so the user's real keyboard/mouse always wins. */
  public notifyRealUserInput(): void {
    if (!this.autoTyping) return;
    this.realUserInputUntil = performance.now() + 1800;
    this.releaseAllVirtualKeys();
  }

  public handleEvent(ev: AgentCompanionEvent): void {
    if (!ev || typeof ev !== 'object') return;
    if (this.privacyPaused) return;

    const now = performance.now();
    const status = (ev.status || 'idle').toLowerCase() as AgentStatusKind;
    const sourceKey = ev.source?.trim().toLowerCase() || 'ai';
    const errorKind = ev.errorKind === 'tool' ? 'tool' : 'task';
    const customText = ev.textIsDefault ? undefined : ev.text;
    for (const [source, error] of this.recentErrors) {
      if (now - error.at > RECOVERY_WINDOW_MS) this.recentErrors.delete(source);
    }
    const previousError = this.recentErrors.get(sourceKey);
    if (ev.kind === 'status' && status === 'error' && this.activeStatus === 'error' &&
        previousError?.kind === errorKind && previousError.tool === ev.tool && previousError.text === customText &&
        now - previousError.at < ERROR_DEDUP_MS) return;

    this.clearTtlTimer();
    if (ev.kind !== 'status' || status !== 'working' || this.recoveryBubble?.source !== sourceKey) {
      this.recoveryBubble = null;
    }
    const target = this.getTarget();
    const is3d = this.is3DTarget();
    const ttl = Math.max(1200, Math.min(300_000, ev.ttlMs ?? 6500));

    if (ev.kind === 'action' && ev.action) {
      this.stopAutoTyping();
      this.activeStatus = 'idle';
      this.dispatchAction(ev.action, target, is3d);
      if (ev.text) {
        this.showBubble(ev.source, ev.text, 'info');
        this.scheduleTtlClear(ttl);
      }
      if (ev.sound && this.allowSound()) {
        this.playNoticeThrottled(ev.action === 'angry' ? 'error' : 'done');
      }
      return;
    }

    if (ev.kind === 'bubble') {
      if (ev.text) {
        this.showBubble(ev.source, ev.text, 'info');
        this.scheduleTtlClear(ttl);
      } else {
        this.hideBubble();
      }
      return;
    }

    this.activeStatus = status;

    switch (status) {
      case 'working': {
        if (ev.autoType !== false) {
          this.startAutoTyping();
        } else {
          this.stopAutoTyping();
        }
        const recovered = this.recentErrors.delete(sourceKey);
        if (recovered) target?.triggerWakeDemo?.();
        const workingText = ev.text || '写代码中…';
        const workingTtl = ev.ttlMs ?? 45_000;
        if (customText) {
          this.recoveryBubble = null;
        } else if (recovered) {
          this.recoveryBubble = { source: sourceKey, text: this.pickCaption('recovery'), until: now + RECOVERY_BUBBLE_MS };
        }
        const recovery = this.recoveryBubble;
        if (recovery && recovery.until > now) {
          const remaining = recovery.until - now;
          this.showBubble(ev.source, recovery.text, 'working');
          if (remaining < workingTtl) {
            this.scheduleTtlClear(remaining, () => {
              if (this.recoveryBubble === recovery && !this.privacyPaused) {
                this.showBubble(ev.source, workingText, 'working');
              }
              this.recoveryBubble = null;
              this.scheduleTtlClear(workingTtl - remaining);
            });
          } else {
            this.scheduleTtlClear(workingTtl);
          }
        } else {
          this.recoveryBubble = null;
          this.showBubble(ev.source, workingText, 'working');
          this.scheduleTtlClear(workingTtl);
        }
        break;
      }
      case 'thinking': {
        if (ev.autoType) {
          this.startAutoTyping();
        } else {
          this.stopAutoTyping();
        }
        target?.triggerCuriousDemo(is3d ? 2.6 : 2600);
        this.showBubble(ev.source, ev.text || '思考中…', 'thinking');
        this.scheduleTtlClear(ev.ttlMs ?? 35_000);
        break;
      }
      case 'waiting': {
        this.stopAutoTyping();
        target?.triggerCuriousDemo(is3d ? 4.2 : 4200);
        this.showBubble(ev.source, ev.text || '等你确认操作', 'waiting');
        this.scheduleTtlClear(ev.ttlMs ?? 90_000);
        if (ev.sound !== false && this.allowSound()) {
          this.playNoticeThrottled('waiting');
        }
        break;
      }
      case 'done': {
        this.recentErrors.delete(sourceKey);
        this.stopAutoTyping();
        if (target?.triggerHappyDemo) {
          target.triggerHappyDemo(is3d ? 2.8 : 2800);
        } else {
          target?.triggerCuriousDemo(is3d ? 2.2 : 2200);
        }
        this.showBubble(ev.source, ev.text || '任务完成！', 'done');
        this.scheduleTtlClear(ev.ttlMs ?? 6_500);
        if (ev.sound !== false && this.allowSound()) {
          this.playNoticeThrottled('done');
        }
        break;
      }
      case 'error': {
        this.stopAutoTyping();
        this.recentErrors.set(sourceKey, { at: now, kind: errorKind, tool: ev.tool, text: customText });
        if (errorKind === 'tool') target?.triggerCuriousDemo(is3d ? 1.5 : 1500);
        else target?.triggerCrossDemo();
        this.showBubble(ev.source, customText || this.pickCaption(errorKind), errorKind === 'tool' ? 'tool-error' : 'error');
        this.scheduleTtlClear(ev.ttlMs ?? 8_500);
        if ((errorKind === 'tool' ? ev.sound === true : ev.sound !== false) && this.allowSound()) {
          this.playNoticeThrottled('error');
        }
        break;
      }
      default: {
        if (status === 'idle') this.recentErrors.delete(sourceKey);
        this.stopAutoTyping();
        if (ev.text) {
          this.showBubble(ev.source, ev.text, 'info');
          this.scheduleTtlClear(ev.ttlMs ?? 3_200);
        } else {
          this.hideBubble();
        }
        break;
      }
    }
  }

  private pickCaption(kind: CaptionKind): string {
    const choices = CAPTIONS[kind].filter(text => text !== this.lastCaption[kind]);
    const text = choices[Math.floor(Math.random() * choices.length)];
    this.lastCaption[kind] = text;
    return text;
  }

  private dispatchAction(
    action: string,
    target: AgentCompanionTarget | null,
    is3d: boolean,
  ): void {
    if (!target) return;
    switch (action.toLowerCase()) {
      case 'happy':
      case 'pet':
        if (target.triggerHappyDemo) {
          target.triggerHappyDemo(is3d ? 2.8 : 2800);
        } else {
          target.triggerCuriousDemo(is3d ? 2.4 : 2400);
        }
        break;
      case 'angry':
        target.triggerCrossDemo();
        break;
      case 'curious':
        target.triggerCuriousDemo(is3d ? 3.5 : 3500);
        break;
      case 'sleep':
        target.triggerSleepDemo();
        break;
      case 'wake':
        if (target.triggerWakeDemo) {
          target.triggerWakeDemo();
        } else {
          target.triggerCuriousDemo(is3d ? 1.5 : 1500);
        }
        break;
      case 'tail_pat':
        if (target.triggerTailPatDemo) {
          target.triggerTailPatDemo();
        } else {
          target.onKeyDown(0x0d, false);
          window.setTimeout(() => target.onKeyUp(0x0d), 140);
        }
        break;
    }
  }

  private playNoticeThrottled(kind: 'done' | 'waiting' | 'error'): void {
    const now = performance.now();
    if (now - this.lastSoundAt < 1200) return;
    this.lastSoundAt = now;
    catAudio.playAgentNotice(kind);
  }

  private startAutoTyping(): void {
    if (this.autoTyping) return;
    this.autoTyping = true;
    this.burstRemaining = 5 + Math.floor(Math.random() * 7);
    this.scheduleNextVirtualKey(40);
  }

  private stopAutoTyping(): void {
    this.autoTyping = false;
    if (this.typeTimer !== null) {
      window.clearTimeout(this.typeTimer);
      this.typeTimer = null;
    }
    this.releaseAllVirtualKeys();
  }

  private releaseAllVirtualKeys(): void {
    for (const id of this.releaseTimers) {
      window.clearTimeout(id);
    }
    this.releaseTimers.clear();
    const target = this.getTarget();
    for (const vk of this.heldVirtualVks) {
      target?.onKeyUp(vk);
    }
    this.heldVirtualVks.clear();
  }

  private scheduleNextVirtualKey(delayMs: number): void {
    if (!this.autoTyping) return;
    if (this.typeTimer !== null) {
      window.clearTimeout(this.typeTimer);
    }
    this.typeTimer = window.setTimeout(() => {
      this.typeTimer = null;
      if (!this.autoTyping) return;

      const now = performance.now();
      if (this.privacyPaused || now < this.realUserInputUntil) {
        this.scheduleNextVirtualKey(260);
        return;
      }

      const target = this.getTarget();
      if (target) {
        const vk = CODING_VKS[Math.floor(Math.random() * CODING_VKS.length)];
        target.onKeyDown(vk, false);
        this.heldVirtualVks.add(vk);
        const holdMs = 55 + Math.floor(Math.random() * 55);
        const rid = window.setTimeout(() => {
          this.releaseTimers.delete(rid);
          if (this.heldVirtualVks.delete(vk)) {
            this.getTarget()?.onKeyUp(vk);
          }
        }, holdMs);
        this.releaseTimers.add(rid);
      }

      this.burstRemaining -= 1;
      if (this.burstRemaining <= 0) {
        this.burstRemaining = 5 + Math.floor(Math.random() * 8);
        this.scheduleNextVirtualKey(260 + Math.floor(Math.random() * 340));
      } else {
        this.scheduleNextVirtualKey(75 + Math.floor(Math.random() * 105));
      }
    }, delayMs);
  }

  private showBubble(
    source: string | undefined,
    text: string,
    tone: 'working' | 'thinking' | 'waiting' | 'done' | 'error' | 'tool-error' | 'info',
  ): void {
    const el = this.bubbleEl;
    if (!el) return;
    const prefix = source && source !== 'AI' && source !== 'MCP' ? `${source} · ` : '';
    const icon =
      tone === 'working'
        ? '💻'
        : tone === 'thinking'
          ? '🧠'
          : tone === 'waiting'
            ? '⏳'
            : tone === 'done'
              ? '✅'
              : tone === 'error'
                ? '💢'
                : tone === 'tool-error' ? '🛠️' : '🐾';
    el.dataset.tone = tone;
    el.textContent = `${icon} ${prefix}${text}`;
    el.classList.remove('hidden');
  }

  private hideBubble(): void {
    if (!this.bubbleEl) return;
    this.bubbleEl.classList.add('hidden');
  }

  private clearTtlTimer(): void {
    if (this.ttlTimer !== null) {
      window.clearTimeout(this.ttlTimer);
      this.ttlTimer = null;
    }
  }

  private scheduleTtlClear(ms: number, next?: () => void): void {
    this.clearTtlTimer();
    this.ttlTimer = window.setTimeout(() => {
      this.ttlTimer = null;
      if (next) {
        next();
        return;
      }
      this.recoveryBubble = null;
      this.activeStatus = 'idle';
      this.stopAutoTyping();
      this.hideBubble();
    }, ms);
  }

  public getStatus(): AgentStatusKind {
    return this.activeStatus;
  }

  public dispose(): void {
    this.recentErrors.clear();
    this.recoveryBubble = null;
    this.clearTtlTimer();
    this.stopAutoTyping();
    this.hideBubble();
  }
}
