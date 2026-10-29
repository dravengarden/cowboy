/** A focused field at the bottom of a long cover sheet stays visible when the
 * keyboard lifts the sheet (synthetic keyboard geometry, not iOS acceptance). */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline, Stack, TextField, Typography } from "@mui/material";
import { DetentSheet } from "@cowboy/app-shell";
import { BrowserProductTheme } from "./browserProductTheme";
import { KEYBOARD_INSET_CHANGED_EVENT } from "./keyboardInset";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { useSheetFocusReveal } from "./sheetFocusReveal";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}
const settle = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function RevealHost(): null {
  useSheetFocusReveal();
  return null;
}

export async function runSheetFocusRevealBrowserChecks(): Promise<void> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const style = document.documentElement.style;
  const previous = style.getPropertyValue("--kb-inset");
  const keyboard = (inset: number) => {
    style.setProperty("--kb-inset", `${inset}px`);
    globalThis.dispatchEvent(new Event(KEYBOARD_INSET_CHANGED_EVENT));
  };
  try {
    keyboard(0);
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <RevealHost />
          <DetentSheet
            open
            onClose={() => {}}
            cover
            ariaLabel="Focus reveal fixture"
            haptic={false}
            animateOnOpen={false}
            footer={
              <MobileDecisionActions
                shelf
                preserveFocus
                confirmLabel="Save"
                onConfirm={() => {}}
                onCancel={() => {}}
              />
            }
          >
            <Stack spacing={2} sx={{ p: 2 }}>
              {Array.from({ length: 14 }, (_, index) => (
                <Typography key={index} sx={{ py: 2 }}>Setting {index + 1}</Typography>
              ))}
              <TextField label="Session title" slotProps={{ htmlInput: { "aria-label": "Session title" } }} />
            </Stack>
          </DetentSheet>
        </BrowserProductTheme>,
      )
    );
    let field: HTMLInputElement | null = null;
    for (let attempt = 0; attempt < 100 && !field; attempt++) {
      field = document.querySelector<HTMLInputElement>('[aria-label="Session title"]');
      if (!field) await settle(20);
    }
    check(field, "focus reveal sheet never mounted");
    const sheet = field.closest<HTMLElement>("[data-detent-sheet='true']")!;
    const footer = sheet.querySelector<HTMLElement>("[data-detent-sheet-footer]")!;
    // The platform reveal has already happened when the sheet shrinks: focus
    // without scrolling, then let the keyboard lift the cover.
    field.focus({ preventScroll: true });
    keyboard(Math.round(globalThis.innerHeight * 0.45));
    await settle(700);
    const box = field.closest<HTMLElement>(".MuiFormControl-root")!.getBoundingClientRect();
    const footerTop = footer.getBoundingClientRect().top;
    const sheetTop = sheet.getBoundingClientRect().top;
    check(
      box.bottom <= footerTop + 0.5 && box.top >= sheetTop,
      `focused sheet field stays between the sheet top and its footer (field ${box.top}–${box.bottom}, footer ${footerTop})`,
    );
    // When the keyboard closes the sheet grows; the field stays in view.
    keyboard(0);
    await settle(700);
    const closed = field.closest<HTMLElement>(".MuiFormControl-root")!.getBoundingClientRect();
    check(
      closed.bottom <= footer.getBoundingClientRect().top + 0.5 &&
        closed.top >= sheet.getBoundingClientRect().top,
      "the field stays visible after the keyboard closes",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    if (previous) style.setProperty("--kb-inset", previous);
    else style.removeProperty("--kb-inset");
    (document.activeElement as HTMLElement | null)?.blur();
  }
}
