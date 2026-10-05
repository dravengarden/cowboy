// Vim for every native Desktop text field (FOCUS.md "Vim in text fields"),
// when the Vim setting is on. CodeMirror editors run the full Vim; titles,
// names, searches and other `<input>`/`<textarea>` fields get this compact
// one from the same physical keys:
//
//   Insert  the field types; `Esc`/`Ctrl-[` enters Normal (cursor steps left).
//   Normal  the field is read-only, so an input method cannot start marked
//           text, and its one-character selection is the block cursor.
//           Motions h l 0 ^ $ w b e W B E f F t T ; , (j k gg G in a
//           textarea); edits x X D C s S r ~ p P, operators d c y with a
//           motion or doubled (dd cc yy), u / Ctrl-R; i a I A insert.
//
// Keys a single-line field cannot use (j, k, Space, Enter, Tab, Esc, and
// digits, `[`, `]`, `g`, `G` inside a dialog) pass through untouched, so the
// surrounding surface keeps its grammar: the dialog moves rows, the leader
// arms, the Draft title returns to its body.

import { getVimSetting } from "../../vimSetting";
import { workspaceCommandKey } from "../commands/workspaceCommandKey";
import {
  findChar,
  firstNonBlank,
  lineEnd,
  lineRange,
  lineStart,
  normalClamp,
  operatorRange,
  removeRange,
  verticalMove,
  type VimRange,
  type VimText,
  wordBackward,
  wordEnd,
  wordForward,
} from "./inputVimEdit";

type Field = HTMLInputElement | HTMLTextAreaElement;

const MODE = "data-vim-input-mode";
/** This layer, not the app, made the field read-only. */
const OWNED_READONLY = "data-vim-readonly";
const TEXT_TYPES = new Set(["", "text", "search", "url", "email", "tel"]);

interface FieldState {
  undo: VimText[];
  redo: VimText[];
  insertStart: VimText | null;
  /** A started command: `d`, `c`, `y`, `f`, `dt`, `r`, `g`, … */
  pending: string;
  lastFind: { kind: "f" | "F" | "t" | "T"; char: string } | null;
}

const states = new WeakMap<Field, FieldState>();
let register: { text: string; linewise: boolean } = { text: "", linewise: false };

function state(field: Field): FieldState {
  let current = states.get(field);
  if (!current) {
    current = { undo: [], redo: [], insertStart: null, pending: "", lastFind: null };
    states.set(field, current);
  }
  return current;
}

/** A field this layer drives: a plain text input or textarea, Vim on. */
export function inputVimField(target: EventTarget | null): Field | null {
  if (!getVimSetting()) return null;
  const field = target instanceof HTMLInputElement
    ? (TEXT_TYPES.has(target.getAttribute("type") ?? "") ? target : null)
    : target instanceof HTMLTextAreaElement
    ? target
    : null;
  if (!field || field.disabled) return null;
  if (field.readOnly && !field.hasAttribute(OWNED_READONLY)) return null;
  if (field.closest(".cm-editor, [data-desktop-vim='off']")) return null;
  return field;
}

export function isInputVimNormal(target: EventTarget | null): boolean {
  return target instanceof Element && target.getAttribute(MODE) === "normal";
}

function snapshot(field: Field): VimText {
  return { value: field.value, cursor: field.selectionStart ?? 0 };
}

function setValue(field: Field, value: string): void {
  if (field.value === value) return;
  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(field) as object,
    "value",
  )?.set;
  setter?.call(field, value);
  // React's onChange listens for `input`; the prototype setter keeps its
  // value tracker from swallowing the change.
  field.dispatchEvent(new Event("input", { bubbles: true }));
}

function place(field: Field, cursor: number): void {
  const value = field.value;
  const at = value.length === 0 ? 0 : normalClamp(value, cursor);
  const end = value.length === 0 || value[at] === "\n" ? at : at + 1;
  field.setSelectionRange(at, end, "forward");
}

function cursorOf(field: Field): number {
  return field.selectionStart ?? 0;
}

/** Enter Normal on `field`, the cursor on `cursor` (default: where it is). */
export function enterInputNormal(field: Field, cursor?: number): void {
  const current = state(field);
  if (
    field.getAttribute(MODE) !== "normal" && current.insertStart &&
    current.insertStart.value !== field.value
  ) {
    current.undo.push(current.insertStart);
    current.redo = [];
  }
  current.insertStart = null;
  current.pending = "";
  if (!field.readOnly) {
    field.setAttribute(OWNED_READONLY, "");
    field.readOnly = true;
  }
  field.setAttribute(MODE, "normal");
  place(field, cursor ?? cursorOf(field));
}

/** Enter Insert with the caret at `at`. */
export function enterInputInsert(field: Field, at: number): void {
  const current = state(field);
  current.insertStart = snapshot(field);
  current.pending = "";
  if (field.hasAttribute(OWNED_READONLY)) {
    field.readOnly = false;
    field.removeAttribute(OWNED_READONLY);
  }
  field.setAttribute(MODE, "insert");
  const caret = Math.max(0, Math.min(at, field.value.length));
  field.setSelectionRange(caret, caret);
}

function edit(field: Field, value: string, cursor: number): void {
  const current = state(field);
  current.undo.push(snapshot(field));
  current.redo = [];
  setValue(field, value);
  place(field, cursor);
}

function yank(text: string, linewise = false): void {
  register = { text, linewise };
  void globalThis.navigator?.clipboard?.writeText(text).catch(() => {});
}

function motionTarget(
  field: Field,
  pos: number,
  key: string,
): number | null {
  const value = field.value;
  switch (key) {
    case "h":
    case "ArrowLeft":
      return Math.max(lineStart(value, pos), pos - 1);
    case "l":
    case "ArrowRight":
      return Math.min(Math.max(lineEnd(value, pos) - 1, pos), pos + 1);
    case "0":
    case "Home":
      return lineStart(value, pos);
    case "^":
      return firstNonBlank(value, pos);
    case "$":
    case "End":
      return Math.max(lineStart(value, pos), lineEnd(value, pos) - 1);
    case "w":
    case "W":
      return wordForward(value, pos, key === "W");
    case "b":
    case "B":
      return wordBackward(value, pos, key === "B");
    case "e":
    case "E":
      return wordEnd(value, pos, key === "E");
    default:
      return null;
  }
}

/** The character a key types, for `f`/`r` arguments. */
function argument(event: KeyboardEvent): string | null {
  if (event.key.length === 1) return event.key;
  const physical = workspaceCommandKey(event);
  if (physical.length === 1) return physical;
  const digit = /^Digit(\d)$/.exec(event.code)?.[1];
  return digit ?? null;
}

const PASS_ALWAYS = new Set(["Escape", "Enter", "Tab", " ", "ArrowUp", "ArrowDown"]);
const PASS_IN_DIALOG = new Set(["1", "2", "3", "4", "5", "6", "7", "8", "9", "[", "]", "g", "G"]);

/**
 * Run one keydown against the focused field. Returns true when consumed.
 * The caller resolves IME ownership first (desktopKeyIntent).
 */
export function handleInputVimKey(event: KeyboardEvent): boolean {
  const field = inputVimField(event.target);
  if (!field) return false;
  const consume = (): true => {
    event.preventDefault();
    event.stopImmediatePropagation();
    return true;
  };
  if (field.getAttribute(MODE) !== "normal") {
    const escape = (event.key === "Escape" && !event.ctrlKey &&
      !event.metaKey && !event.altKey && !event.shiftKey) ||
      (event.ctrlKey && !event.metaKey && !event.altKey &&
        event.code === "BracketLeft");
    if (!escape) return false;
    const at = field.selectionStart ?? 0;
    enterInputNormal(field, at > lineStart(field.value, at) ? at - 1 : at);
    return consume();
  }
  const current = state(field);
  if (event.metaKey || event.altKey) return false;
  if (event.ctrlKey) {
    if (event.code === "KeyR" && !event.shiftKey) {
      const next = current.redo.pop();
      if (!next) return consume();
      current.undo.push(snapshot(field));
      setValue(field, next.value);
      place(field, next.cursor);
      return consume();
    }
    return false;
  }
  if (event.repeat && current.pending) return consume();
  const key = workspaceCommandKey(event);
  const value = field.value;
  const pos = cursorOf(field);
  const multiline = field instanceof HTMLTextAreaElement;
  const pending = current.pending;
  current.pending = "";

  // Arguments: f/F/t/T, an operator's f/t, and r.
  if (/[fFtT]$/.test(pending) && pending.length <= 2) {
    const char = argument(event);
    if (char === null) return consume();
    const kind = pending.at(-1) as "f" | "F" | "t" | "T";
    current.lastFind = { kind, char };
    const target = findChar(value, pos, kind, char);
    if (target === null) return consume();
    const operator = pending.length === 2 ? pending[0] as "d" | "c" | "y" : null;
    if (operator) {
      applyOperator(field, operator, operatorRange(value, pos, kind, target, operator));
    } else place(field, target);
    return consume();
  }
  if (pending === "r") {
    const char = argument(event);
    if (char === null || value.length === 0) return consume();
    edit(field, value.slice(0, pos) + char + value.slice(pos + 1), pos);
    return consume();
  }
  if (pending === "g") {
    if (key === "g" && multiline) place(field, 0);
    return consume();
  }

  const inDialog = field.closest("[role='dialog']") !== null;
  if (!pending) {
    if (PASS_ALWAYS.has(key) && !(multiline && (key === "ArrowUp" || key === "ArrowDown"))) {
      return false;
    }
    if (!multiline && (key === "j" || key === "k")) return false;
    if (inDialog && !multiline && PASS_IN_DIALOG.has(key)) return false;
  }

  // Operators.
  if (pending === "d" || pending === "c" || pending === "y") {
    const operator = pending;
    if (key === operator) {
      applyOperator(field, operator, lineRange(value, pos));
      return consume();
    }
    if (/^[fFtT]$/.test(key)) {
      current.pending = operator + key;
      return consume();
    }
    const target = motionTarget(field, pos, key);
    if (target === null) return consume();
    applyOperator(field, operator, operatorRange(value, pos, key, target, operator));
    return consume();
  }

  if (multiline && (key === "j" || key === "k" || key === "ArrowDown" || key === "ArrowUp")) {
    const target = verticalMove(value, pos, key === "j" || key === "ArrowDown" ? 1 : -1);
    if (target !== null) place(field, target);
    return consume();
  }
  const motion = motionTarget(field, pos, key);
  if (motion !== null) {
    place(field, motion);
    return consume();
  }
  switch (key) {
    case "i":
      enterInputInsert(field, pos);
      return consume();
    case "a":
      enterInputInsert(field, value.length === 0 ? 0 : Math.min(pos + 1, lineEnd(value, pos)));
      return consume();
    case "I":
      enterInputInsert(field, firstNonBlank(value, pos));
      return consume();
    case "A":
      enterInputInsert(field, lineEnd(value, pos));
      return consume();
    case "x":
    case "X": {
      const from = key === "x" ? pos : pos - 1;
      if (from < lineStart(value, pos) || from >= lineEnd(value, pos)) return consume();
      yank(value.slice(from, from + 1));
      edit(field, value.slice(0, from) + value.slice(from + 1), from);
      return consume();
    }
    case "D":
    case "C": {
      applyOperator(field, key === "D" ? "d" : "c", { from: pos, to: lineEnd(value, pos) });
      return consume();
    }
    case "s":
      applyOperator(field, "c", { from: pos, to: Math.min(pos + 1, lineEnd(value, pos)) });
      return consume();
    case "S":
      applyOperator(field, "c", lineRange(value, pos));
      return consume();
    case "~": {
      const char = value[pos];
      if (char === undefined || char === "\n") return consume();
      const swapped = char === char.toUpperCase() ? char.toLowerCase() : char.toUpperCase();
      edit(field, value.slice(0, pos) + swapped + value.slice(pos + 1), pos + 1);
      return consume();
    }
    case "p":
    case "P": {
      if (!register.text) return consume();
      if (register.linewise && multiline) {
        const at = key === "p" ? lineEnd(value, pos) : lineStart(value, pos);
        const text = key === "p" ? `\n${register.text}` : `${register.text}\n`;
        edit(field, value.slice(0, at) + text + value.slice(at), key === "p" ? at + 1 : at);
        return consume();
      }
      const at = key === "p" && value.length > 0 ? pos + 1 : pos;
      edit(field, value.slice(0, at) + register.text + value.slice(at), at + register.text.length - 1);
      return consume();
    }
    case "u": {
      const previous = current.undo.pop();
      if (!previous) return consume();
      current.redo.push(snapshot(field));
      setValue(field, previous.value);
      place(field, previous.cursor);
      return consume();
    }
    case ";":
    case ",": {
      const find = current.lastFind;
      if (!find) return consume();
      const reverse = { f: "F", F: "f", t: "T", T: "t" } as const;
      const kind = key === ";" ? find.kind : reverse[find.kind];
      const target = findChar(value, pos, kind, find.char);
      if (target !== null) place(field, target);
      return consume();
    }
    case "G":
      if (multiline) place(field, lineStart(value, value.length));
      return consume();
    case "d":
    case "c":
    case "y":
    case "f":
    case "F":
    case "t":
    case "T":
    case "r":
    case "g":
      current.pending = key;
      return consume();
  }
  // Any other character is not text in Normal.
  return event.key.length === 1 ? consume() : false;
}

function applyOperator(
  field: Field,
  operator: "d" | "c" | "y",
  range: VimRange,
): void {
  const value = field.value;
  const text = value.slice(range.from, range.to);
  yank(text, range.linewise === true);
  if (operator === "y") {
    place(field, range.from);
    return;
  }
  const next = operator === "c" && range.linewise
    ? value.slice(0, range.from) + value.slice(range.to)
    : removeRange(value, range);
  const at = operator === "c" || !range.linewise
    ? range.from
    : Math.min(range.from, Math.max(0, next.length - 1));
  edit(field, next, at);
  if (operator === "c") enterInputInsert(field, range.from);
}

/**
 * Keep the mode honest across focus: a field focused by a click or Tab
 * types (Insert); leaving it drops this layer's read-only flag. A click in
 * Normal moves the block cursor there.
 */
export function installInputVim(): () => void {
  const onFocusIn = (event: FocusEvent): void => {
    const field = inputVimField(event.target);
    if (field && field.getAttribute(MODE) === null) {
      field.setAttribute(MODE, "insert");
      state(field).insertStart = snapshot(field);
    }
  };
  const onFocusOut = (event: FocusEvent): void => {
    const field = event.target;
    if (!(field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement)) return;
    if (field.hasAttribute(OWNED_READONLY)) {
      field.readOnly = false;
      field.removeAttribute(OWNED_READONLY);
    }
    field.removeAttribute(MODE);
    const current = states.get(field);
    if (current) {
      current.pending = "";
      if (current.insertStart && current.insertStart.value !== field.value) {
        current.undo.push(current.insertStart);
      }
      current.insertStart = null;
    }
  };
  const onMouseUp = (event: MouseEvent): void => {
    const field = event.target;
    if (
      (field instanceof HTMLInputElement || field instanceof HTMLTextAreaElement) &&
      isInputVimNormal(field) && field.selectionStart === field.selectionEnd
    ) {
      place(field, field.selectionStart ?? 0);
    }
  };
  document.addEventListener("focusin", onFocusIn, true);
  document.addEventListener("focusout", onFocusOut, true);
  document.addEventListener("mouseup", onMouseUp, true);
  return () => {
    document.removeEventListener("focusin", onFocusIn, true);
    document.removeEventListener("focusout", onFocusOut, true);
    document.removeEventListener("mouseup", onMouseUp, true);
  };
}
