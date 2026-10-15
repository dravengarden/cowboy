// User Vim configuration: a small, honest subset of a vimrc. Each line is a
// map command or `set timeoutlen=N`; `"` starts a comment. CodeMirror editors
// apply every mapping through codemirror-vim; native text fields (inputVim)
// honour the insert-mode escape sequences (`inoremap jj <Esc>`), which is the
// one mapping they can express without a full Vim keymap.

export type VimMapMode = "normal" | "visual" | "insert" | "operatorPending";

export interface VimMapping {
  readonly lhs: string;
  readonly rhs: string;
  /** undefined = every mode (`map` / `noremap`). */
  readonly mode: VimMapMode | undefined;
  readonly recursive: boolean;
}

export interface VimConfigError {
  readonly line: number;
  readonly text: string;
  readonly message: string;
}

export interface VimConfig {
  readonly mappings: readonly VimMapping[];
  /** Vim's `timeoutlen`: how long a mapped key sequence waits for its next key. */
  readonly timeoutMs: number;
  readonly errors: readonly VimConfigError[];
}

/** Vim's own default. codemirror-vim's 200 ms is too short for a typed `jj`. */
export const DEFAULT_VIM_TIMEOUT_MS = 1000;
const MAX_TIMEOUT_MS = 5000;

const MAP_COMMANDS: Record<string, { mode: VimMapMode | undefined; recursive: boolean }> = {
  map: { mode: undefined, recursive: true },
  noremap: { mode: undefined, recursive: false },
  nmap: { mode: "normal", recursive: true },
  nnoremap: { mode: "normal", recursive: false },
  vmap: { mode: "visual", recursive: true },
  vnoremap: { mode: "visual", recursive: false },
  xmap: { mode: "visual", recursive: true },
  xnoremap: { mode: "visual", recursive: false },
  imap: { mode: "insert", recursive: true },
  inoremap: { mode: "insert", recursive: false },
  omap: { mode: "operatorPending", recursive: true },
  onoremap: { mode: "operatorPending", recursive: false },
};

/** Ready-made lines the settings offer for leaving Insert without Esc. */
export const VIM_ESCAPE_PRESETS = ["jj", "jk", "kj"] as const;

export function escapeMappingLine(sequence: string): string {
  return `inoremap ${sequence} <Esc>`;
}

export function parseVimConfig(source: string): VimConfig {
  const mappings: VimMapping[] = [];
  const errors: VimConfigError[] = [];
  let timeoutMs = DEFAULT_VIM_TIMEOUT_MS;
  source.split(/\r?\n/).forEach((raw, index) => {
    const text = raw.trim().replace(/^:+/, "");
    if (text === "" || text.startsWith('"')) return;
    const fail = (message: string): void => {
      errors.push({ line: index + 1, text: raw.trim(), message });
    };
    const [command = "", ...rest] = text.split(/\s+/);
    if (command === "set") {
      const option = /^(?:timeoutlen|tm)=(\d+)$/.exec(rest.join(" "));
      if (!option) {
        fail("Only `set timeoutlen=<ms>` is supported");
        return;
      }
      const value = Number(option[1]);
      if (value < 50 || value > MAX_TIMEOUT_MS) {
        fail(`timeoutlen must be between 50 and ${String(MAX_TIMEOUT_MS)} ms`);
        return;
      }
      timeoutMs = value;
      return;
    }
    const spec = MAP_COMMANDS[command];
    if (!spec) {
      fail(`Unsupported command \`${command}\``);
      return;
    }
    // `<silent>`, `<buffer>` … have no meaning here; skip them like Vim does.
    const args = rest.filter((arg) => !/^<(silent|buffer|nowait|expr|unique)>$/i.test(arg));
    if (args.length < 2) {
      fail("A mapping needs a key sequence and its replacement");
      return;
    }
    const [lhs, ...rhsParts] = args as [string, ...string[]];
    mappings.push({ lhs, rhs: rhsParts.join(" "), mode: spec.mode, recursive: spec.recursive });
  });
  return { mappings, timeoutMs, errors };
}

const ESCAPE_RHS = /^(<esc>|<c-\[>)$/i;

/** Plain-character insert sequences that leave Insert, e.g. `jj`. */
export function insertEscapeSequences(config: VimConfig): string[] {
  return config.mappings
    .filter((mapping) =>
      (mapping.mode === "insert" || mapping.mode === undefined) &&
      ESCAPE_RHS.test(mapping.rhs) &&
      // Special keys (<C-c>, <Tab>) are not typed characters a text field
      // could undo; they already have native meanings there.
      mapping.lhs.length >= 2 && !mapping.lhs.includes("<")
    )
    .map((mapping) => mapping.lhs);
}

const PRESET_LINE = new RegExp(
  `^\\s*inoremap\\s+(${VIM_ESCAPE_PRESETS.join("|")})\\s+<esc>\\s*$`,
  "i",
);

/** The escape preset a config currently uses, if any. */
export function selectedEscapePreset(source: string): string | null {
  for (const line of source.split(/\r?\n/)) {
    const match = PRESET_LINE.exec(line);
    if (match) return match[1]!.toLowerCase();
  }
  return null;
}

/** Replace the config's preset escape line with `preset` (or remove it). */
export function withEscapePreset(source: string, preset: string | null): string {
  const kept = source.split(/\r?\n/).filter((line) => !PRESET_LINE.test(line));
  while (kept.length > 0 && kept[0]!.trim() === "") kept.shift();
  const lines = preset === null ? kept : [escapeMappingLine(preset), ...kept];
  return lines.join("\n");
}

/** A typed-character tracker for one field: decides when the last key
 *  completes an escape sequence. Characters before the last one were already
 *  inserted and must be removed by the caller. */
export class InsertEscapeMatcher {
  private typed = "";
  private at = 0;

  constructor(private readonly now: () => number = () => performance.now()) {}

  reset(): void {
    this.typed = "";
  }

  /** Returns the matched sequence when `key` completes one within the
   *  timeout, else records the key as a possible prefix and returns null. */
  push(key: string, sequences: readonly string[], timeoutMs: number): string | null {
    const time = this.now();
    const fresh = time - this.at <= timeoutMs ? this.typed : "";
    const candidate = fresh + key;
    const complete = sequences.find((sequence) => candidate.endsWith(sequence));
    if (complete && candidate.length >= complete.length && fresh.length > 0) {
      this.typed = "";
      return complete;
    }
    // Keep the longest suffix that is still a prefix of some sequence.
    let kept = "";
    for (let start = 0; start < candidate.length; start++) {
      const suffix = candidate.slice(start);
      if (sequences.some((sequence) => sequence.startsWith(suffix))) {
        kept = suffix;
        break;
      }
    }
    this.typed = kept;
    this.at = time;
    return null;
  }
}
