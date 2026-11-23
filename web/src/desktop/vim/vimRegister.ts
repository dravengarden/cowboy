/**
 * Shared unnamed Vim register bridged between native text fields (`inputVim.ts`)
 * and `@replit/codemirror-vim` (`imeAutoInsertVim.ts`).
 *
 * Kept free of `@replit/codemirror-vim` imports so mobile/touch bundles never
 * pull the CodeMirror Vim runtime; `imeAutoInsertVim.ts` connects the live
 * `RegisterController` once the Desktop Vim chunk loads.
 */
export interface VimRegisterValue {
  readonly text: string;
  readonly linewise: boolean;
}

export interface VimRegisterController {
  read(): VimRegisterValue;
  write(text: string, linewise: boolean): void;
}

let fallback: VimRegisterValue = { text: "", linewise: false };
let controller: VimRegisterController | null = null;

/**
 * Connect (or disconnect) CodeMirror Vim's `RegisterController`. If native
 * fields already wrote to the fallback register before the Vim chunk loaded,
 * seed the live controller when its unnamed register is still empty.
 */
export function connectVimRegisterController(
  next: VimRegisterController | null,
): void {
  controller = next;
  if (next && fallback.text.length > 0 && next.read().text.length === 0) {
    next.write(fallback.text, fallback.linewise);
  }
}

/**
 * Read the shared unnamed register for native field pasting (`p`/`P`).
 * `@replit/codemirror-vim` stores linewise yanks with a trailing `\n`; strip
 * that single trailing newline so `inputVim` inserts cleanly without doubling
 * blank lines.
 */
export function readVimRegister(): VimRegisterValue {
  const raw = controller ? controller.read() : fallback;
  const text = raw.linewise && raw.text.endsWith("\n")
    ? raw.text.slice(0, -1)
    : raw.text;
  return { text, linewise: raw.linewise };
}

/** Write the shared unnamed register from a native field yank/delete or copy. */
export function writeVimRegister(text: string, linewise = false): void {
  fallback = { text, linewise };
  controller?.write(text, linewise);
}
