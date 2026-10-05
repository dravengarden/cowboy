import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline, Dialog, Stack, TextField } from "@mui/material";
import { useState } from "react";
import { BrowserProductTheme } from "../browserProductTheme";
import { SurfaceProvider } from "../surface/SurfaceProfile";
import { getVimSetting, setVimSetting } from "../vimSetting";
import { DesktopWorkspaceProvider } from "./DesktopWorkspaceController";
import { DesktopCommandProvider } from "./commands/DesktopCommandProvider";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 40));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

function codeFor(key: string): string {
  if (/^[a-z]$/i.test(key)) return `Key${key.toUpperCase()}`;
  if (/^\d$/.test(key)) return `Digit${key}`;
  return ({ "$": "Digit4", "^": "Digit6", "~": "Backquote" } as Record<string, string>)[key] ?? key;
}

/** One keydown as a browser delivers it; returns whether it was consumed. */
function press(key: string, init: KeyboardEventInit = {}): boolean {
  const target = document.activeElement ?? document.body;
  const event = new KeyboardEvent("keydown", {
    key,
    code: codeFor(key),
    shiftKey: /^[A-Z$^~]$/.test(key),
    bubbles: true,
    cancelable: true,
    ...init,
  });
  flushSync(() => target.dispatchEvent(event));
  return event.defaultPrevented;
}

function Fields({ dialog }: { dialog: boolean }): React.JSX.Element {
  const [first, setFirst] = useState("hello world foo");
  const [second, setSecond] = useState("second");
  const [notes, setNotes] = useState("ab\ncde");
  const [open, setOpen] = useState(true);
  const fields = (
    <Stack spacing={2} sx={{ p: 2, width: 360 }}>
      <TextField
        value={first}
        onChange={(event) => setFirst(event.target.value)}
        slotProps={{ htmlInput: { "data-field": "first" } }}
      />
      <TextField
        value={second}
        onChange={(event) => setSecond(event.target.value)}
        slotProps={{ htmlInput: { "data-field": "second" } }}
      />
      <TextField
        multiline
        value={notes}
        onChange={(event) => setNotes(event.target.value)}
        slotProps={{ htmlInput: { "data-field": "notes" } }}
      />
    </Stack>
  );
  return dialog
    ? (
      <Dialog open={open} onClose={() => setOpen(false)}>
        <div data-dialog-open>{fields}</div>
      </Dialog>
    )
    : fields;
}

/** Vim Normal/Insert in native Desktop fields (inputVim), Vim setting on. */
export async function checkInputVim(): Promise<string> {
  const vimBefore = getVimSetting();
  setVimSetting(true);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const field = (name: string) =>
    document.querySelector<HTMLInputElement | HTMLTextAreaElement>(
      `[data-field="${name}"]`,
    )!;
  const mount = (dialog: boolean) =>
    flushSync(() =>
      root.render(
        <SurfaceProvider>
          <BrowserProductTheme>
            <CssBaseline />
            <DesktopWorkspaceProvider>
              <DesktopCommandProvider>
                <Fields dialog={dialog} />
              </DesktopCommandProvider>
            </DesktopWorkspaceProvider>
          </BrowserProductTheme>
        </SurfaceProvider>,
      )
    );
  const mode = (el: HTMLElement): string | undefined => el.dataset.vimInputMode;
  const at = (el: HTMLInputElement | HTMLTextAreaElement) => el.selectionStart;
  const text = (el: HTMLInputElement | HTMLTextAreaElement) => el.value;
  try {
    mount(false);
    await tick();
    const first = field("first");
    first.focus();
    first.setSelectionRange(first.value.length, first.value.length);
    check(mode(first) === "insert", "A focused field types (Insert)");
    check(
      !press("Escape", { isComposing: true }) &&
        mode(first) === "insert",
      "Esc during composition stays with the IME",
    );
    check(press("Escape"), "Esc is consumed by the field");
    check(
      mode(first) === "normal" && first.readOnly &&
        at(first) === 14 && first.selectionEnd === 15,
      "Esc enters Normal with a block cursor one character left",
    );
    press("b");
    check(at(first) === 12, "b moves to the word start");
    press("d");
    press("w");
    await tick();
    check(text(first) === "hello world ", "dw deletes the word");
    press("u");
    await tick();
    check(text(first) === "hello world foo", "u undoes");
    press("0");
    press("w");
    check(at(first) === 6, "0 then w reaches the second word");
    press("c");
    press("w");
    await tick();
    check(
      text(first) === "hello  foo" && mode(first) === "insert" &&
        !first.readOnly && at(first) === 6,
      "cw changes the word and types",
    );
    press("Escape");
    press("u");
    await tick();
    check(text(first) === "hello world foo", "u undoes the change");
    press("$");
    check(at(first) === 14, "$ reaches the last character");
    press("x");
    await tick();
    check(text(first) === "hello world fo", "x deletes under the cursor");
    press("p");
    await tick();
    check(text(first) === "hello world foo", "p puts the deleted character back");
    check(!press("j"), "j passes through a single-line field");
    press("A");
    check(
      mode(first) === "insert" &&
        at(first) === first.value.length,
      "A appends",
    );
    press("Escape");
    // Textarea: j/k move lines.
    const notes = field("notes");
    notes.focus();
    notes.setSelectionRange(1, 1);
    press("Escape");
    check(at(notes) === 0, "Esc in a textarea steps left");
    press("j");
    check(at(notes) === 3, "j moves to the next line");
    press("k");
    check(at(notes) === 0, "k moves back up");
    // In a dialog j/k leave the field for the modal rows, Esc closes.
    flushSync(() => root.render(null));
    mount(true);
    await tick();
    const dialogFirst = field("first");
    dialogFirst.focus();
    press("Escape");
    check(
      mode(dialogFirst) === "normal" &&
        document.querySelector("[data-dialog-open]"),
      "Esc in a dialog field enters Normal and keeps the dialog",
    );
    press("j");
    await tick();
    const dialogSecond = field("second");
    check(
      document.activeElement === dialogSecond &&
        mode(dialogSecond) === "normal",
      "j moves to the next field in Normal",
    );
    press("h");
    check(at(dialogSecond) === 4, "h moves inside the field");
    press("Escape");
    await new Promise((resolve) => setTimeout(resolve, 600));
    check(!document.querySelector("[data-dialog-open]"), "Esc in Normal closes the dialog");
    return "Vim in native Desktop fields: Insert/Normal with a block cursor, motions, operators, undo, put, textarea lines; dialog rows and Esc layering";
  } finally {
    setVimSetting(vimBefore);
    root.unmount();
    container.remove();
  }
}
