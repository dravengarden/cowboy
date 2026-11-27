// IME-aware Insert-mode escape matching (`inoremap jj <Esc>`, `jk`, `kj`, …).
//
// Under macOS/Linux/Windows CJK input methods (WeType, Pinyin, Sogou, Rime),
// pressing physical `j` twice in Insert mode starts an IME composition (`j` →
// `j'j`) before JavaScript `keydown` can prevent it, so codemirror-vim never
// sees the typed letters. This tracker watches physical keys and composition
// updates from the start of a fresh composition and signals an escape abort as
// soon as a configured insert-escape sequence completes within `timeoutlen`.
//
// Multi-letter pinyin that does not start with an escape prefix (e.g. `ni'j'j`
// or `jie'jue`), edits that leave the prefix (e.g. `ji`, candidate selection,
// Backspace), and slow pauses past `timeoutlen` disqualify the composition so
// normal CJK typing is preserved.

import { isImeKeyEvent } from "../../imeKey";
import { getVimConfigSource } from "../../vimSetting";
import { insertEscapeSequences, parseVimConfig } from "./vimConfig";

export interface InsertEscapeConfig {
  readonly sequences: readonly string[];
  readonly timeoutMs: number;
}

let cachedConfig: { source: string; value: InsertEscapeConfig } | null = null;

/** Cached insert-mode escape sequences and timeout from the user's Vim config. */
export function getInsertEscapeConfig(): InsertEscapeConfig {
  const source = getVimConfigSource();
  if (cachedConfig?.source !== source) {
    const config = parseVimConfig(source);
    cachedConfig = {
      source,
      value: {
        sequences: insertEscapeSequences(config),
        timeoutMs: config.timeoutMs,
      },
    };
  }
  return cachedConfig.value;
}

/** Physical ASCII letter from `event.code`, ignoring IME `Process`/229 key text. */
export function physicalAsciiLetter(
  event: Pick<KeyboardEvent, "code" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey">,
): string | null {
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  const match = /^Key([A-Z])$/.exec(event.code);
  if (!match?.[1]) return null;
  const letter = match[1].toLowerCase();
  return event.shiftKey ? letter.toUpperCase() : letter;
}

/** Normalize IME pre-edit text (`j'j`, `j j`, full-width `ｊｊ`) for prefix checks. */
export function normalizeImeCompositionText(text: string): string {
  let out = "";
  for (const ch of text) {
    if (ch === "'" || ch === "\u2019" || ch === "\u200b" || /\s/.test(ch)) continue;
    const code = ch.charCodeAt(0);
    if (code >= 0xff21 && code <= 0xff3a) {
      out += String.fromCharCode(code - 0xff21 + 0x41);
    } else if (code >= 0xff41 && code <= 0xff5a) {
      out += String.fromCharCode(code - 0xff41 + 0x61);
    } else {
      out += ch;
    }
  }
  return out;
}

export type ImeEscapeKeyEvent = Pick<
  KeyboardEvent,
  "code" | "key" | "keyCode" | "isComposing" | "ctrlKey" | "metaKey" | "altKey" | "shiftKey"
>;

export type ImeEscapeKeyAction = "none" | "arm" | "escape";

const START_BRIDGE_MS = 250;

/** Per-editor/field tracker for IME Insert-mode escape sequences. */
export class ImeInsertEscapeTracker {
  private pendingStart: { keys: string; at: number; disqualified: boolean } | null = null;
  private active: { keys: string; lastKeyAt: number; disqualified: boolean } | null = null;

  constructor(private readonly now: () => number = () => performance.now()) {}

  reset(): void {
    this.pendingStart = null;
    this.active = null;
  }

  onKeyDown(
    event: ImeEscapeKeyEvent,
    composing: boolean,
    sequences: readonly string[],
    timeoutMs: number,
  ): ImeEscapeKeyAction {
    if (sequences.length === 0) {
      this.reset();
      return "none";
    }
    const imeSignal = composing || isImeKeyEvent(event);
    if (!imeSignal) {
      this.reset();
      return "none";
    }
    const time = this.now();
    const letter = physicalAsciiLetter(event);

    if (composing) {
      if (!this.active) {
        if (this.pendingStart && time - this.pendingStart.at <= START_BRIDGE_MS) {
          this.active = {
            keys: this.pendingStart.keys,
            lastKeyAt: this.pendingStart.at,
            disqualified: this.pendingStart.disqualified,
          };
          this.pendingStart = null;
        } else {
          this.active = { keys: "", lastKeyAt: time, disqualified: false };
        }
      }
      if (this.active.disqualified) return "none";
      if (letter === null) {
        this.active.disqualified = true;
        return "none";
      }
      if (this.active.keys.length > 0 && time - this.active.lastKeyAt > timeoutMs) {
        this.active.disqualified = true;
        return "none";
      }
      const candidate = this.active.keys + letter;
      if (sequences.includes(candidate)) {
        this.active.disqualified = true;
        this.pendingStart = null;
        return "escape";
      }
      if (sequences.some((seq) => seq.startsWith(candidate))) {
        this.active.keys = candidate;
        this.active.lastKeyAt = time;
        return candidate.length === 1 ? "arm" : "none";
      }
      this.active.disqualified = true;
      return "none";
    }

    // Not composing yet, but key carries an IME marker (`Process`/229): typically
    // the first key of a fresh composition, arriving just before `compositionstart`.
    if (letter === null) {
      this.pendingStart = { keys: "", at: time, disqualified: true };
      return "none";
    }
    if (
      this.pendingStart &&
      !this.pendingStart.disqualified &&
      this.pendingStart.keys.length > 0 &&
      time - this.pendingStart.at <= timeoutMs
    ) {
      const candidate = this.pendingStart.keys + letter;
      if (sequences.includes(candidate)) {
        this.pendingStart = null;
        this.active = { keys: candidate, lastKeyAt: time, disqualified: true };
        return "escape";
      }
      if (sequences.some((seq) => seq.startsWith(candidate))) {
        this.pendingStart = { keys: candidate, at: time, disqualified: false };
        return "none";
      }
    }
    if (sequences.some((seq) => seq.startsWith(letter))) {
      this.pendingStart = { keys: letter, at: time, disqualified: false };
      return "arm";
    }
    this.pendingStart = { keys: "", at: time, disqualified: true };
    return "none";
  }

  /** Returns true when this fresh composition may still complete an escape sequence. */
  onCompositionStart(): boolean {
    const time = this.now();
    if (this.pendingStart && time - this.pendingStart.at <= START_BRIDGE_MS) {
      this.active = {
        keys: this.pendingStart.keys,
        lastKeyAt: this.pendingStart.at,
        disqualified: this.pendingStart.disqualified,
      };
      this.pendingStart = null;
      return !this.active.disqualified;
    }
    this.pendingStart = null;
    this.active = { keys: "", lastKeyAt: time, disqualified: false };
    return true;
  }

  /** Catch compositions (`j'j`) even when an IME or CDP driver omits `KeyJ` on keydown. */
  onCompositionUpdate(
    data: string,
    sequences: readonly string[],
    timeoutMs: number,
  ): "none" | "escape" {
    if (sequences.length === 0 || !this.active || this.active.disqualified) return "none";
    const normalized = normalizeImeCompositionText(data);
    if (normalized.length === 0) return "none";
    const time = this.now();
    if (this.active.keys.length > 0 && time - this.active.lastKeyAt > timeoutMs) {
      this.active.disqualified = true;
      return "none";
    }
    if (sequences.includes(normalized)) {
      this.active.disqualified = true;
      return "escape";
    }
    if (sequences.some((seq) => seq.startsWith(normalized))) {
      if (normalized.length > this.active.keys.length) {
        this.active.keys = normalized;
        this.active.lastKeyAt = time;
      }
      return "none";
    }
    // Inline-preview CJK/Japanese/Zhuyin IMEs may emit non-ASCII pre-edit text
    // (`就`, `じ`, `ㄨ`) right after a valid physical prefix key. Keep tracking
    // physical keys rather than disqualifying; ASCII pinyin (`ji`, `wo`) still
    // disqualifies immediately.
    if (this.active.keys.length > 0 && /[^A-Za-z]/.test(normalized)) {
      return "none";
    }
    this.active.disqualified = true;
    return "none";
  }

  onCompositionEnd(): void {
    this.active = null;
    this.pendingStart = null;
  }
}
