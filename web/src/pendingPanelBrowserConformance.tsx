import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { PendingPanel } from "./Composer";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { composerStackExpandedStore } from "./composerStackAccordion";
import { MessagePreview } from "./MessagePreview";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 60));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

/** Real pending panels: density, touch targets and exclusive disclosure. */
export async function checkPendingPanelLayout(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const expanded = composerStackExpandedStore();
  const previous = expanded.get();
  const originalFont = document.documentElement.style.fontSize;
  try {
    for (const desktop of [true, false]) {
      for (const fontSize of [8, 10.4, 16, 24]) {
        document.documentElement.style.fontSize = `${fontSize}px`;
        for (const width of [320, 430, 600, 1200]) {
          container.style.width = `${width}px`;
          expanded.set(null);
          flushSync(() =>
            root.render(
              <SurfaceProvider>
                <BrowserProductTheme>
                  <CssBaseline />
                  <DesktopWorkspaceProvider>
                    <DesktopCommandProvider>
                      {(["queued", "draft"] as const).map((kind) => (
                        <PendingPanel
                          key={kind}
                          desktop={desktop}
                          keyboardOpen={false}
                          kind={kind}
                          sessionId="pending-layout-fixture"
                          items={[1, 2].map((n) => ({
                            id: `${kind}-${n}`,
                            text: `Prompt ${n}`,
                            attachments: [],
                          }))}
                          status="running"
                          commands={() => []}
                        />
                      ))}
                    </DesktopCommandProvider>
                  </DesktopWorkspaceProvider>
                </BrowserProductTheme>
              </SurfaceProvider>,
            )
          );
          await tick();
          for (const label of ["Expand queued messages", "Expand drafts"]) {
            // Existing descriptive accessibility names remain stable.
            const button = container.querySelector<HTMLButtonElement>(
              `button[aria-label='${label}']`,
            );
            check(button, `Disclosure exists: ${label}`);
            const header = button.parentElement!;
            const panel = header.parentElement!;
            check(
              getComputedStyle(panel).borderTopWidth ===
                (desktop ? "0px" : "1px"),
              "Only Desktop removes the card frame",
            );
            if (!desktop) {
              check(
                parseFloat(getComputedStyle(panel).borderTopLeftRadius) > 0,
                "Mobile retains rounded card",
              );
              check(
                button.textContent?.includes(
                  label.includes("drafts") ? "2 Drafts" : "2 Queued Messages",
                ),
                "Mobile retains count-first labels",
              );
              check(
                getComputedStyle(panel).backgroundColor !== "rgba(0, 0, 0, 0)",
                "Mobile retains staging fill",
              );
            }
            check(
              getComputedStyle(panel).boxShadow === "none",
              "Panel has no focus halo",
            );
            check(
              header.scrollWidth <= header.clientWidth + 1,
              `Header fits ${width}px at ${fontSize}px`,
            );
            const actions = header.querySelectorAll<HTMLButtonElement>(
              "button",
            );
            for (const action of actions) {
              if (action.getClientRects().length === 0) continue;
              const rect = action.getBoundingClientRect();
              check(
                rect.height >= (desktop ? 32 : action === button ? 44 : 28),
                "Desktop targets and original Mobile disclosure/icon sizing are retained",
              );
              check(
                rect.right <= header.getBoundingClientRect().right + 1,
                "Actions stay inside header",
              );
            }
            check(
              Boolean(header.querySelector("button[aria-label='reorder']")) ===
                !desktop,
              "Only Desktop hides collapsed reorder",
            );
          }
          const checkGrip = async (disclosure: HTMLButtonElement): Promise<void> => {
            const panel = disclosure.parentElement!.parentElement!;
            const reorder = panel.querySelector<HTMLButtonElement>("button[aria-label='reorder']");
            if (reorder && reorder.getClientRects().length > 0) {
              reorder.click();
              await tick();
            }
            const grips = [...panel.querySelectorAll<HTMLButtonElement>("button[aria-label='Drag to reorder']")]
              .filter((grip) => grip.getClientRects().length > 0);
            check(grips.length === 2, "Both expanded rows expose their reorder grip");
            for (const grip of grips) {
              // Desktop: a slim edge handle (1.25rem × 2rem); Mobile: 44px.
              const width = desktop ? 1.25 * fontSize : 44;
              const height = desktop ? 2 * fontSize : 44;
              const rect = grip.getBoundingClientRect();
              const slot = grip.parentElement!.getBoundingClientRect();
              check(Math.abs(rect.width - width) < 1 && Math.abs(rect.height - height) < 1,
                "Desktop grip scales with the root font; Mobile keeps its touch target");
              check(Math.abs(slot.width - width) < 1 &&
                (desktop || Math.abs(slot.height - height) < 1),
                "Grip and its leading slot have matching geometry");
              if (desktop) {
                check(grip.scrollWidth <= grip.clientWidth + 1,
                  `Desktop grip fits at ${fontSize}px root font`);
              }
            }
          };
          const drafts = container.querySelector<HTMLButtonElement>(
            "button[aria-label='Expand drafts']",
          )!;
          drafts.click();
          await tick();
          check(
            expanded.get() === "draft",
            "Draft disclosure expands without delivering",
          );
          check(
            drafts.getAttribute("aria-expanded") === "true",
            "Disclosure announces expansion",
          );
          await checkGrip(drafts);
          const queue = container.querySelector<HTMLButtonElement>(
            "button[aria-label='Expand queued messages']",
          )!;
          queue.click();
          await tick();
          check(
            expanded.get() === "queued" &&
              drafts.getAttribute("aria-expanded") === "false",
            "Queue replaces Drafts exclusively",
          );
          await checkGrip(queue);
          const menu = container.querySelector<HTMLButtonElement>(
            "button[aria-label='Queue actions']",
          )!;
          menu.click();
          await tick();
          check(
            expanded.get() === "queued",
            "Menu does not collapse the panel",
          );
          document.dispatchEvent(
            new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
          );
          // Unmount closes the portaled menu before the next width.
          flushSync(() => root.render(null));
        }
      }
    }
    return "Queue/Drafts fit 320–1200px at normal/enlarged fonts on both surfaces; Mobile card/labels/reorder preserved, disclosure, menu and touch targets passed";
  } finally {
    expanded.set(previous);
    document.documentElement.style.fontSize = originalFont;
    root.unmount();
    container.remove();
  }
}

/** Show more / less commits a stationary touch on pointerup (WebKit may drop
 *  the click after scroll momentum) without letting the paired click undo it. */
export async function checkPendingPreviewTouchDisclosure(): Promise<string> {
  const container = document.createElement("div");
  container.style.width = "320px";
  document.body.append(container);
  const root = createRoot(container);
  const text = Array.from({ length: 12 }, (_, n) => `Line ${n + 1} of a long draft`)
    .join("\n");
  const opened: string[] = [];
  const disclosure = (): HTMLButtonElement => {
    const button = [...container.querySelectorAll("button")].find((b) =>
      /Show (more|less)/.test(b.textContent ?? "")
    );
    check(button, "Disclosure exists");
    return button;
  };
  const touch = (type: string, button: HTMLElement, dy = 0): void => {
    const rect = button.getBoundingClientRect();
    button.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        pointerId: 7,
        pointerType: "touch",
        isPrimary: true,
        clientX: rect.left + rect.width / 2,
        clientY: rect.top + rect.height / 2 + dy,
      }),
    );
  };
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <MessagePreview text={text} onClick={() => opened.push("edit")} />
        </BrowserProductTheme>,
      )
    );
    await tick();
    check(disclosure().textContent?.includes("Show more"), "Long draft clamps");

    // Momentum case: pointerup arrives, the compatibility click never does.
    touch("pointerdown", disclosure());
    touch("pointerup", disclosure());
    await tick();
    check(disclosure().textContent?.includes("Show less"), "Touch expands on pointerup");

    // A late paired click is consumed instead of collapsing again.
    disclosure().dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }),
    );
    await tick();
    check(disclosure().textContent?.includes("Show less"), "Paired click is consumed");

    // A scroll gesture over the control stays native and does not toggle.
    touch("pointerdown", disclosure());
    touch("pointermove", disclosure(), 40);
    touch("pointerup", disclosure(), 40);
    await tick();
    check(disclosure().textContent?.includes("Show less"), "Scroll does not toggle");

    // Mouse and keyboard keep the ordinary click path.
    disclosure().dispatchEvent(
      new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse", isPrimary: true }),
    );
    disclosure().dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }),
    );
    await tick();
    check(disclosure().textContent?.includes("Show more"), "Mouse click collapses");
    disclosure().click();
    await tick();
    check(disclosure().textContent?.includes("Show less"), "Keyboard click expands");
    check(opened.length === 0, "Disclosure never opens the card's edit");
    return "Pending preview Show more toggles once per stationary touch, consumes its paired click, ignores scrolls, and keeps mouse/keyboard clicks";
  } finally {
    root.unmount();
    container.remove();
  }
}
