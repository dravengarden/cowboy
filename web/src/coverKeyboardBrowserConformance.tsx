/** Actual cover/footer layout with synthetic keyboard geometry, not iOS IME acceptance. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { DetentSheet } from "@cowboy/app-shell";
import { MobileDecisionActions } from "./MobileDecisionActions";
import { inferKeyboardOpen } from "./keyboardGeometry";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}
export async function runCoverKeyboardBrowserConformance(): Promise<string[]> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const style = document.documentElement.style;
  const previous = style.getPropertyValue("--kb-inset");
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
          <DetentSheet
            open
            onClose={() => {}}
            cover
            keyboardOpen={keyboardOpen}
            ariaLabel="Keyboard fixture"
            haptic={false}
            animateOnOpen={false}
            footer={
              <MobileDecisionActions
                shelf
                flat
                confirmLabel="Create"
                onConfirm={() => {}}
                onCancel={() => {}}
              />
            }
          >
            <input aria-label="Title" defaultValue="New session" />
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
    return [
      "overlay and resized keyboards retain identical compact footer spacing",
      "keyboard dismissal restores home-indicator clearance",
      "flat New session actions have no gradient riser or button shadow",
    ];
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    style.setProperty("--kb-inset", previous);
  }
}
