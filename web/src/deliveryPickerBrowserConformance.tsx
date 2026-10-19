import { useState } from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { DeliveryPicker } from "./DeliveryPicker";
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
  let setQueued!: (count: number) => void;
  let setPaused!: (paused: boolean) => void;
  let finishFirstRead!: () => void;
  let reads = 0;
  globalThis.fetch = async () => {
    if (reads++ === 0) {
      await new Promise<void>((resolve) => finishFirstRead = resolve);
    }
    return Response.json({
      target: { worker_epoch: "worker-1", turn_id: "turn-1" },
      steer: true,
      controls: { version: 1, next_checkpoint: true },
      status: "busy",
      paused: false,
      hold: null,
      policy: { mode: "steer", revision: 0 },
    });
  };
  function Harness() {
    const [anchor, setAnchor] = useState<HTMLElement | null>(null);
    const [busy, updateBusy] = useState(true);
    const [queueCount, updateQueueCount] = useState(0);
    const [paused, updatePaused] = useState(false);
    setBusy = updateBusy;
    setQueued = updateQueueCount;
    setPaused = updatePaused;
    return (
      <BrowserProductTheme>
        <button
          style={{ position: "fixed", bottom: 20, right: 20 }}
          onClick={(event) => setAnchor(event.currentTarget)}
        >
          Send…
        </button>
        <DeliveryPicker
          sessionId="fixture"
          anchor={anchor}
          busy={busy}
          paused={paused}
          queueCount={queueCount}
          onClose={() => {
            calls.push("close");
            setAnchor(null);
          }}
          onNextTurn={() => calls.push("next")}
          onQueueFront={() => calls.push("front")}
          onSteer={(target, checkpoint) =>
            calls.push(
              `${checkpoint ? "checkpoint" : "steer"}:${target.turn_id}`,
            )}
          onSendAndPause={() => calls.push("pause")}
          onStop={() => calls.push("stop")}
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
    document.activeElement.closest('[role="menu"]')?.getAttribute(
        "aria-busy",
      ) === "false";
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
    const initialSteer = document.querySelector<HTMLElement>(
      '[data-delivery-command="steer"]',
    )!;
    check(
      initialSteer.getAttribute("aria-disabled") === "true",
      "Steer is disabled during capability discovery",
    );
    check(
      !initialSteer.classList.contains("Mui-selected"),
      "An unavailable default never looks selected",
    );
    check(
      getComputedStyle(initialSteer).color !== getComputedStyle(
        document.querySelector<HTMLElement>('[data-delivery-command="next"]')!,
      ).color,
      "Unavailable Steer is visually distinct from actionable rows",
    );
    initialSteer.click();
    check(calls.length === 0, "Loading Steer cannot submit a message");
    check(
      initialSteer.textContent?.includes("Checking runtime"),
      "A disabled Steer explains why",
    );
    check(
      !document.querySelector(
        '[data-delivery-command="interrupt"]',
      ),
      "Delivery menu omits interrupt",
    );
    finishFirstRead();
    for (let attempt = 0; attempt < 50 && !ready(); attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(
      document.activeElement?.getAttribute("data-delivery-command") === "steer",
      "Default delivery receives focus",
    );
    check(
      !document.querySelector("[data-delivery-picker] .Mui-selected"),
      "Actions have no selected state",
    );
    check(
      document.querySelectorAll("[data-delivery-picker] .MuiListItemIcon-root")
        .length === 5,
      "Every action has an icon",
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
    check(
      !initialSteer.classList.contains("Mui-selected"),
      "An expired Steer loses its selected appearance",
    );
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
    const front = document.querySelector<HTMLElement>(
      '[data-delivery-command="front"]',
    )!;
    check(
      front.getAttribute("aria-disabled") === "true",
      "Empty queue cannot be jumped",
    );
    flushSync(() => {
      setQueued(2);
      setPaused(true);
    });
    front.focus();
    key("Enter");
    check(
      calls.at(-1) === "front",
      "Keyboard chooses queue first without an active turn",
    );
    check(
      front.textContent?.includes("Queue stays paused"),
      "Queue first preserves Pause",
    );
    flushSync(() => setQueued(0));
    const beforeEmptyFront = calls.length;
    key("Enter");
    check(
      calls.length === beforeEmptyFront,
      "A drained queue invalidates focused queue first",
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
      !calls.some((call) => call.startsWith("interrupt")),
      "Alt Enter cannot interrupt",
    );
    const paper = document.querySelector<HTMLElement>("[data-delivery-picker]")!
      .closest<HTMLElement>(".MuiPaper-root")!;
    await new Promise((resolve) => setTimeout(resolve, 300));
    const beforeAnchorLoss = paper.getBoundingClientRect();
    const send = host.querySelector<HTMLButtonElement>("button")!;
    send.style.display = "none";
    window.visualViewport?.dispatchEvent(new Event("resize"));
    await new Promise((resolve) => setTimeout(resolve, 100));
    const afterAnchorLoss = paper.getBoundingClientRect();
    check(
      Math.abs(beforeAnchorLoss.top - afterAnchorLoss.top) < 2 &&
        Math.abs(beforeAnchorLoss.left - afterAnchorLoss.left) < 2,
      "Hiding the send anchor does not move the popup to the screen corner",
    );
    const pause = document.querySelector<HTMLElement>(
      '[data-delivery-command="pause"]',
    )!;
    pause.focus();
    key("Enter");
    check(calls.at(-1) === "pause", "Send & pause is keyboard reachable");
    return "Delivery picker preserves exact target, IME/repeat exclusion, disabled focus and explicit next-turn selection";
  } finally {
    flushSync(() => root.unmount());
    host.remove();
    globalThis.fetch = originalFetch;
  }
}
