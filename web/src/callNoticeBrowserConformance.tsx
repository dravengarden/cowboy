import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { CssBaseline } from "@mui/material";
import { PendingPanel } from "./Composer";
import { PromptOriginNote } from "./PromptOriginNote";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { composerStackExpandedStore } from "./composerStackAccordion";
import { closeBackgroundTasks, inspectorState } from "./backgroundTaskInspectorState";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 60));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

const CALL = "call-75a40bd32bda4b984d2d6809";
const NOTICE = `[Cowboy] Managed call ${CALL} (Stormbird code-health plan, round 2 (after Astra Max findings)) finished: completed. Read it with cowboy_read: cowboy.calls.result("${CALL}",{offset:0,chars:4000}). A running, failed or partial review is never evidence of a clean review.`;
const FAILED = "[Cowboy] Managed call call-2 finished: failed, error provider_unavailable. Read it with cowboy_read: x";

/** Managed-call callbacks: one line in the queue and transcript, visibly not a
 *  person's prompt, and a tap opens that call instead of the row editor. */
export async function checkCallNoticeLayout(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const expanded = composerStackExpandedStore();
  const previous = expanded.get();
  const originalFont = document.documentElement.style.fontSize;
  try {
    for (const desktop of [true, false]) {
      for (const fontSize of [10.4, 16, 24]) {
        document.documentElement.style.fontSize = `${fontSize}px`;
        for (const width of [320, 600]) {
          container.style.width = `${width}px`;
          expanded.set(null);
          flushSync(() =>
            root.render(
              <SurfaceProvider>
                <BrowserProductTheme>
                  <CssBaseline />
                  <DesktopWorkspaceProvider>
                    <DesktopCommandProvider>
                      <PendingPanel
                        desktop={desktop}
                        keyboardOpen={false}
                        kind="queued"
                        sessionId="notice-queue"
                        items={[
                          { id: "q-1", text: NOTICE, cmid: `call-notice:${CALL}`, attachments: [] },
                          { id: "q-2", text: NOTICE, attachments: [] },
                        ]}
                        status="busy"
                        commands={() => []}
                      />
                      <div data-transcript-session="notice-transcript">
                        <PromptOriginNote
                          origin={{ actor: "cowboy", source: "managed_call" }}
                          chunks={[{ type: "text", text: FAILED }]}
                          provider="claude-code"
                        />
                      </div>
                    </DesktopCommandProvider>
                  </DesktopWorkspaceProvider>
                </BrowserProductTheme>
              </SurfaceProvider>,
            )
          );
          await tick();
          const disclosure = container.querySelector<HTMLButtonElement>(
            "button[aria-label='Expand queued messages']",
          );
          if (disclosure?.getAttribute("aria-expanded") !== "true") disclosure?.click();
          await tick();
          const lines = [...container.querySelectorAll<HTMLElement>("[data-call-notice]")];
          check(lines.length === 2, `Queue and transcript callbacks render (${String(lines.length)})`);
          const [queued, transcript] = lines as [HTMLElement, HTMLElement];
          for (const line of lines) {
            const rect = line.getBoundingClientRect();
            check(
              rect.height >= 44 && rect.height <= Math.max(64, 4 * fontSize),
              `Callback keeps a readable two-line touch target at ${String(width)}px/${String(fontSize)}px (${String(rect.height)})`,
            );
            check(line.scrollWidth <= line.clientWidth + 1, "Callback never overflows sideways");
          }
          check(queued.dataset["callNoticeTone"] === "success", "Completed call reads as success");
          check(getComputedStyle(queued).borderLeftWidth === "0px", "Embedded callback does not nest another card");
          check(transcript.dataset["callNoticeTone"] === "error", "Failed call reads as error");
          check(transcript.textContent?.includes("Failed · provider unavailable"), "Failure keeps its code");
          check(queued.textContent?.includes("Stormbird code-health plan"), "Queue shows the call title");
          check(!queued.textContent?.includes("cowboy_read"), "Agent instruction stays out of the line");
          const queueRows = queued.closest("[data-pending-edit-target]");
          check(queueRows && !queueRows.querySelector(".cm-editor"), "Callback row skips the Markdown preview");
          // A Cowboy cmid is required: identical text a person queued keeps its preview.
          check(container.querySelectorAll(".cm-editor").length >= 1, "Human row keeps the Markdown preview");
          queued.click();
          await tick();
          check(
            inspectorState().sessionId === "notice-queue" && inspectorState().key === `call:${CALL}`,
            "Queue callback opens its call",
          );
          check(!container.querySelector("[data-pending-edit-target] [contenteditable='true']"), "Tap does not open the editor");
          closeBackgroundTasks();
          transcript.click();
          check(
            inspectorState().sessionId === "notice-transcript" && inspectorState().key === "call:call-2",
            "Transcript callback opens the call of its transcript",
          );
          closeBackgroundTasks();
        }
      }
    }
    return "Managed-call callbacks share readable activity rows and open their call";
  } finally {
    flushSync(() => root.render(null));
    root.unmount();
    container.remove();
    expanded.set(previous);
    document.documentElement.style.fontSize = originalFont;
  }
}
