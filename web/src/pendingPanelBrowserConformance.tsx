import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { PendingPanel } from "./Composer";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { composerStackExpandedStore } from "./composerStackAccordion";

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
      for (const fontSize of [16, 24]) {
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
              getComputedStyle(panel).borderTopWidth === "0px",
              "Panel has no card frame",
            );
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
              const rect = action.getBoundingClientRect();
              check(
                rect.height >= (desktop ? 32 : 44),
                "Disclosure and menu retain target height",
              );
              check(
                rect.right <= header.getBoundingClientRect().right + 1,
                "Actions stay inside header",
              );
            }
            check(
              !header.querySelector("button[aria-label='reorder']"),
              "Collapsed rail hides reorder",
            );
          }
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
    return "Queue/Drafts fit 320–1200px at normal/enlarged fonts on both surfaces; disclosure, menu and touch targets passed";
  } finally {
    expanded.set(previous);
    document.documentElement.style.fontSize = originalFont;
    root.unmount();
    container.remove();
  }
}
