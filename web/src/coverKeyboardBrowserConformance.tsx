/** Actual cover/footer layout with synthetic keyboard geometry, not iOS IME acceptance. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { DetentSheet, MobileSheetActionGroup, MobileSheetDismiss } from "@cowboy/app-shell";
import { MobileSheetBackDismiss } from "../../components/app-shell/bottom-sheet";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { inferKeyboardOpen } from "./keyboardGeometry";
import { ObsidianSheet } from "./ObsidianSheet";
import { runLightboxStageBrowserChecks } from "./lightboxStageBrowserChecks";
import { runUndoHistoryBrowserChecks } from "./undoHistoryBrowserChecks";
export { renderUndoHistoryPreview } from "./undoHistoryBrowserChecks";
import { Button, CssBaseline, Stack, TextField, Typography } from "@mui/material";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}
export async function runCoverKeyboardBrowserConformance(keepFixture = false): Promise<string[]> {
  if (!keepFixture) await runLightboxStageBrowserChecks();
  if (!keepFixture) await runUndoHistoryBrowserChecks();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const style = document.documentElement.style;
  const previous = style.getPropertyValue("--kb-inset");
  let confirmed = 0;
  let cancelled = 0;
  function render(
    layoutHeight: number,
    visualHeight: number,
    focused: boolean,
    inset: number,
  ) {
    style.setProperty("--kb-inset", `${inset}px`);
    const keyboardOpen = inferKeyboardOpen({
      layoutHeight,
      visualHeight,
      baselineHeight: 844,
      editableFocused: focused,
    });
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <DetentSheet
            open
            onClose={() => {}}
            cover
            keyboardOpen={keyboardOpen}
            ariaLabel="Keyboard fixture"
            haptic={false}
            animateOnOpen={false}
            header={<Typography sx={{ px: 2, pb: 1, fontWeight: 700 }}>Create</Typography>}
            footer={
              <MobileDecisionActions
                shelf
                flat
                preserveFocus
                keyboardOpen={keyboardOpen}
                confirmLabel="Create"
                onConfirm={() => { confirmed++; }}
                onCancel={() => { cancelled++; }}
              />
            }
          >
            <Stack spacing={2} sx={{ p: 2 }}>
              <TextField label="Title" slotProps={{ htmlInput: { "aria-label": "Title" } }} defaultValue="New session" helperText="Clear to auto-name from the first message" />
              <TextField label="Project" value="Hawk/columbus" slotProps={{ input: { readOnly: true } }} />
            </Stack>
          </DetentSheet>
        </BrowserProductTheme>,
      )
    );
  }
  function footer() {
    const node = document.querySelector<HTMLElement>(
      '[aria-label="Keyboard fixture"] [data-detent-sheet-footer]',
    );
    check(node, "missing actual footer");
    return node;
  }
  try {
    render(844, 430, true, 414);
    await new Promise((resolve) => setTimeout(resolve, 40));
    check(
      getComputedStyle(footer()).paddingBottom === "8px",
      "overlay keyboard needs only compact spacing",
    );
    // A cover sheet mounts its body two frames after opening (DetentSheet
    // contentReady). Wait for that instead of a fixed delay: frames slip on a
    // loaded host, which made this fixture fail intermittently.
    let title: HTMLInputElement | null = null;
    for (let attempt = 0; attempt < 100 && !title; attempt++) {
      title = document.querySelector<HTMLInputElement>('[aria-label="Title"]');
      if (!title) await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(title, "cover sheet body never mounted");
    title.focus();
    title.value = "Keep my draft";
    await new Promise((resolve) => setTimeout(resolve, 40));
    const hide = footer().querySelector<HTMLButtonElement>('[aria-label="Hide keyboard"]')!;
    check(hide.getBoundingClientRect().width >= 44, "keyboard dismiss must be visible with a thumb-sized target");
    const down = new PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerType: "touch" });
    hide.dispatchEvent(down);
    check(down.defaultPrevented && document.activeElement === title, "pointer-down must preserve the input until activation");
    hide.click();
    await new Promise((resolve) => setTimeout(resolve, 40));
    check(document.activeElement !== title && title.value === "Keep my draft", "keyboard dismissal must blur without losing the draft");
    check(confirmed === 0 && cancelled === 0, "keyboard dismissal must not submit or cancel");
    check(hide.getBoundingClientRect().width === 0, "keyboard utility must disappear after editing ends");
    render(430, 430, true, 0);
    check(
      getComputedStyle(footer()).paddingBottom === "8px",
      "resized keyboard restored home-indicator padding",
    );
    render(844, 844, false, 0);
    check(
      parseFloat(getComputedStyle(footer()).paddingBottom) >= 16,
      "dismissal lost bottom safe spacing",
    );
    check(
      getComputedStyle(footer().closest("[data-detent-sheet]")!).boxShadow ===
        "none",
      "cover casts a shadow into the keyboard gap",
    );
    const shelf = footer().querySelector<HTMLElement>(
      "[data-mobile-decision-footer-shelf]",
    )!;
    check(getComputedStyle(shelf).boxShadow === "none", "shelf has a shadow");
    check(
      getComputedStyle(shelf, "::before").content === "none",
      "shelf retains the gradient riser",
    );
    const button = shelf.querySelector<HTMLElement>(".MuiButton-contained")!;
    check(
      getComputedStyle(button).boxShadow === "none",
      "confirm retains its glow",
    );
    check(button.getBoundingClientRect().height >= 44, "confirm target shrank");
    check(getComputedStyle(button).transform === "none", "decision motion must not move the target");
    check(getComputedStyle(button, "::after").transitionProperty === "opacity", "decision feedback must use the shared state layer");
    const beforePress = button.getBoundingClientRect();
    button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, isPrimary: true, button: 0, pointerId: 91 }));
    // React's event update and the CSS transition need a painted frame; under
    // concurrent builds, a fixed delay can expire before the first paint.
    for (let attempt = 0; attempt < 50 && Number(getComputedStyle(button, "::after").opacity) < 0.1; attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(Number(getComputedStyle(button, "::after").opacity) >= 0.1, "touch press feedback must survive focus-preserving pointer-down");
    check(button.getBoundingClientRect().y === beforePress.y, "press must not displace the target");
    document.dispatchEvent(new PointerEvent("pointercancel", { pointerId: 91 }));
    check(!button.hasAttribute("data-decision-pressed") && confirmed === 0, "cancelled pointer must clear feedback without submitting");
    const vv = window.visualViewport!;
    const heightDescriptor = Object.getOwnPropertyDescriptor(vv, "height");
    Object.defineProperty(vv, "height", { configurable: true, value: window.innerHeight - 300 });
    try {
      for (const actions of [<><Button>Cancel</Button><Button variant="contained">Save</Button></>, undefined]) {
        flushSync(() => root.render(
          <BrowserProductTheme>
            <div role="dialog" aria-label="Underlying form">
            <MobileDecisionActions keyboardOpen confirmLabel="Underlying save" onConfirm={() => { confirmed++; }} onCancel={() => { cancelled++; }} />
            <ObsidianSheet open title="Rename" onClose={() => { cancelled++; }} actions={actions}>
              <input aria-label="Rename title" defaultValue="Retain me" />
            </ObsidianSheet>
            </div>
          </BrowserProductTheme>,
        ));
        const rename = document.querySelector<HTMLInputElement>('[aria-label="Rename title"]')!;
        rename.focus();
        vv.dispatchEvent(new Event("resize"));
        await new Promise((resolve) => setTimeout(resolve, 100));
        const utilities = [...document.querySelectorAll<HTMLButtonElement>('[aria-label="Hide keyboard"]')].filter((node) => node.getBoundingClientRect().width > 0);
        check(utilities.length === 1, "compact forms must have exactly one keyboard utility with or without actions");
        utilities[0]!.click();
        check(document.activeElement !== rename && rename.value === "Retain me" && cancelled === 0, "compact keyboard dismissal must preserve the sheet and value");
      }
    } finally {
      if (heightDescriptor) Object.defineProperty(vv, "height", heightDescriptor);
      else Reflect.deleteProperty(vv, "height");
    }
    // Use the real hit-test tree: dispatching synthetic events directly to a
    // body would bypass the transparent overlay that caused this regression.
    let closed = 0;
    let ancestorClicks = 0;
    const onClose = () => { closed++; };
    const content = <div style={{ height: 2400 }}>Long scrollable settings</div>;
    for (const cover of [false, true]) {
      for (const footer of [
        <MobileSheetDismiss onClose={onClose} />,
        <MobileSheetBackDismiss onBack={() => {}} onClose={onClose} />,
        <MobileSheetActionGroup actions={[{ key: "close", label: "Close", icon: "×", onPress: onClose }]} />,
      ]) {
        flushSync(() => root.render(
          <BrowserProductTheme>
            <CssBaseline />
            <div onClick={() => { ancestorClicks++; }}>
              <DetentSheet open onClose={onClose} cover={cover} footerOverlay footer={footer} haptic={false} animateOnOpen={false}>
                {content}
              </DetentSheet>
            </div>
          </BrowserProductTheme>,
        ));
        await new Promise((resolve) => setTimeout(resolve, 100));
        await checkFooterHitTargets();
      }
    }
    flushSync(() => root.render(
      <BrowserProductTheme>
        <CssBaseline />
        <ObsidianSheet open onClose={onClose} floatingDismiss>{content}</ObsidianSheet>
      </BrowserProductTheme>,
    ));
    await new Promise((resolve) => setTimeout(resolve, 500));
    await checkFooterHitTargets();
    async function checkFooterHitTargets() {
      const button = container.querySelector<HTMLButtonElement>('button[aria-label="Close"]');
      check(button, "missing floating close control");
      const rect = button.getBoundingClientRect();
      const dialog = button.closest<HTMLElement>('[role="dialog"]')!;
      const bounds = dialog.getBoundingClientRect();
      for (const x of [bounds.left + 25, bounds.right - 25]) {
        const hit = document.elementFromPoint(x, rect.y + rect.height / 2);
        check(hit && dialog.contains(hit), "footer gap must hit the sheet content");
        let scrollport: Element | null = hit;
        while (scrollport && scrollport !== dialog && getComputedStyle(scrollport).overflowY !== "auto") {
          scrollport = scrollport.parentElement;
        }
        check(scrollport && scrollport !== dialog && scrollport.scrollHeight > scrollport.clientHeight,
          "both footer gaps must reach an overflowing native scrollport");
      }
      const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
      check(hit && button.contains(hit), "floating control must retain its hit target");
      const before = closed;
      hit.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
      await new Promise((resolve) => setTimeout(resolve, 400));
      check(closed === before + 1 && ancestorClicks === 0, "close must activate exactly once without reaching its owner");
    }
    return [
      "overlay and resized keyboards retain identical compact footer spacing",
      "keyboard dismissal restores home-indicator clearance",
      "form actions use stationary state feedback; keyboard dismissal preserves the form without submitting or cancelling",
      "cover, content and compact sheet footer gaps reach the native scrollport while close, back and action islands retain exclusive hits",
    ];
  } finally {
    if (keepFixture) {
      render(844, 430, true, 0);
      document.querySelector<HTMLInputElement>('[aria-label="Title"]')?.focus();
    } else {
      flushSync(() => root.unmount());
      container.remove();
      style.setProperty("--kb-inset", previous);
    }
  }
}
