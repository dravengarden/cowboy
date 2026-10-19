import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { DeliveryPicker } from "./DeliveryPicker";
import { InterruptDeliveryConfirm } from "./InterruptDeliveryConfirm";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";

export async function checkDeliveryPicker(): Promise<string> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  const originalFetch = globalThis.fetch;
  const calls: string[] = [];
  let setBusy!: (busy: boolean) => void;
  globalThis.fetch = async () =>
    Response.json({
      target: { worker_epoch: "worker-1", turn_id: "turn-1" },
      steer: true,
      controls: { version: 1, next_checkpoint: true },
      status: "busy",
      paused: false,
      hold: null,
      policy: { mode: "steer", revision: 0 },
    });
  function Harness() {
    const [anchor, setAnchor] = useState<HTMLElement | null>(null);
    const [busy, updateBusy] = useState(true);
    const [confirm, setConfirm] = useState(false);
    setBusy = updateBusy;
    return (
      <BrowserProductTheme>
        <button onClick={(event) => setAnchor(event.currentTarget)}>
          Send…
        </button>
        <DeliveryPicker
          sessionId="fixture"
          anchor={anchor}
          busy={busy}
          paused={false}
          queueCount={0}
          onClose={() => {
            calls.push("close");
            setAnchor(null);
          }}
          onNextTurn={() => calls.push("next")}
          onSteer={(target, checkpoint) =>
            calls.push(
              `${checkpoint ? "checkpoint" : "steer"}:${target.turn_id}`,
            )}
          onInterrupt={(target) => {
            calls.push(`interrupt:${target.turn_id}`);
            setAnchor(null);
            setConfirm(true);
          }}
          onPause={() => calls.push("pause")}
          onStop={() => calls.push("stop")}
        />
        <InterruptDeliveryConfirm
          open={confirm}
          preview="Replacement input"
          onClose={() => setConfirm(false)}
          onConfirm={async () => {
            calls.push("confirmed");
            setConfirm(false);
          }}
        />
      </BrowserProductTheme>
    );
  }
  const check = (value: unknown, message: string): void => {
    if (!value) throw new Error(message);
  };
  const key = (key: string, extra: KeyboardEventInit = {}): void => {
    flushSync(() =>
      document.activeElement?.dispatchEvent(
        new KeyboardEvent("keydown", {
          key,
          bubbles: true,
          cancelable: true,
          ...extra,
        }),
      )
    );
  };
  const ready = (): boolean =>
    document.activeElement?.getAttribute("data-delivery-command") === "steer" &&
    document.activeElement.getAttribute("aria-disabled") !== "true" &&
    Boolean(
      document.activeElement.closest('[role="menu"]')?.textContent?.includes(
        "Choose delivery timing",
      ),
    );
  try {
    flushSync(() =>
      root.render(
        <SurfaceProvider>
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Harness />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </SurfaceProvider>,
      )
    );
    flushSync(() => host.querySelector("button")!.click());
    for (let attempt = 0; attempt < 50 && !ready(); attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(
      document.activeElement?.getAttribute("data-delivery-command") === "steer",
      "Default delivery receives focus",
    );
    key("Enter", { repeat: true });
    key("Enter", { isComposing: true });
    check(calls.length === 0, "Held Enter and IME cannot deliver");
    key("Enter");
    check(
      calls.at(-1) === "steer:turn-1",
      "Explicit activation retains captured target",
    );
    const before = calls.length;
    flushSync(() => setBusy(false));
    key("Enter");
    check(
      calls.length === before,
      "Expired focused mode does not fall through",
    );
    key("ArrowDown");
    key("Enter");
    check(
      calls.at(-1) === "next",
      "User can explicitly choose next turn after expiration",
    );
    key("Escape");
    check(calls.at(-1) === "close", "Escape closes only picker");
    flushSync(() => {
      setBusy(true);
      host.querySelector("button")!.click();
    });
    for (let attempt = 0; attempt < 50 && !ready(); attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    key("Enter", { altKey: true });
    check(
      calls.at(-1) === "interrupt:turn-1",
      "Alt Enter opens confirmation for the captured turn",
    );
    for (
      let attempt = 0;
      attempt < 50 && !document.querySelector('[role="dialog"]');
      attempt++
    ) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    const confirmButton = [
      ...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button'),
    ]
      .find((button) => button.textContent?.trim() === "Interrupt & send");
    check(confirmButton, "Interrupt confirmation is a real dialog");
    confirmButton!.focus();
    key("Enter");
    key("Enter", { ctrlKey: true, repeat: true });
    key("Enter", { ctrlKey: true, isComposing: true });
    check(
      !calls.includes("confirmed"),
      "Confirmation rejects bare, held and IME Enter",
    );
    key("Enter", { ctrlKey: true });
    check(
      calls.filter((value) => value === "confirmed").length === 1,
      "Fresh Mod Enter confirms exactly once",
    );
    return "Delivery picker preserves exact target, IME/repeat exclusion, disabled focus and explicit next-turn selection";
  } finally {
    flushSync(() => root.unmount());
    host.remove();
    globalThis.fetch = originalFetch;
  }
}
