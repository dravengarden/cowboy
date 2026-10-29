import { useState } from "react";
import { Menu, MenuItem } from "@mui/material";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { DeliveryPicker } from "./DeliveryPicker";
import { SurfaceContext, type SurfaceProfile } from "./surface/SurfaceProfile";

/** A Desktop profile regardless of the test browser's pointer devices. */
const DESKTOP: SurfaceProfile = {
  kind: "desktop",
  input: "keyboard-pointer",
  touchCapable: false,
  finePointer: true,
  hover: true,
};
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";

/** The live session view offers Steer at once: a slow or failing options
 *  read never shows it as unavailable. */
async function checkLiveSteer(
  root: ReturnType<typeof createRoot>,
  check: (value: unknown, message: string) => void,
  key: (key: string, extra?: KeyboardEventInit) => void,
): Promise<void> {
  const steered: string[] = [];
  let reads = 0;
  let mode: "hang" | "fail-once" = "hang";
  globalThis.fetch = async () => {
    reads++;
    if (mode === "hang") return await new Promise<Response>(() => {});
    if (reads === 1) throw new TypeError("network");
    return Response.json({
      target: { worker_epoch: "w", turn_id: "live" },
      steer: true,
      controls: { version: 1, next_checkpoint: false },
      status: "busy",
      paused: false,
      hold: null,
      policy: { mode: "steer", revision: 0 },
    });
  };
  const open = (session: string): void => {
    const anchor = document.createElement("button");
    document.body.append(anchor);
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={DESKTOP}>
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <BrowserProductTheme>
                <DeliveryPicker
                  sessionId={session}
                  anchor={anchor}
                  busy
                  steerReady
                  paused={false}
                  queueCount={0}
                  onClose={() => {}}
                  onNextTurn={() => {}}
                  onQueueFront={() => {}}
                  onSteer={(target) => steered.push(target?.turn_id ?? "capture")}
                  onSendAndPause={() => {}}
                  onStop={() => {}}
                />
              </BrowserProductTheme>
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </SurfaceContext.Provider>,
      )
    );
  };
  const steerRow = () =>
    document.querySelector<HTMLElement>('[data-delivery-command="steer"]');
  open("live-hang");
  for (let attempt = 0; attempt < 50 && document.activeElement !== steerRow(); attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  check(
    steerRow()?.getAttribute("aria-disabled") !== "true" &&
      document.activeElement === steerRow(),
    "Steer is offered and focused while the options read is still pending",
  );
  check(
    !steerRow()?.textContent?.includes("unavailable"),
    "A pending read never reads as unavailable",
  );
  key("Enter");
  check(steered.at(-1) === "capture", "Steer without a fresh target captures it on submit");
  mode = "fail-once";
  reads = 0;
  flushSync(() => root.render(<></>));
  open("live-retry");
  for (let attempt = 0; attempt < 150 && reads < 2; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  await new Promise((resolve) => setTimeout(resolve, 100));
  check(reads >= 2, "a failed options read is retried");
  check(
    !document.querySelector("[data-delivery-picker]")?.textContent?.includes("Runtime unavailable"),
    "a transient failure never shows Runtime unavailable",
  );
  key("Enter");
  check(steered.at(-1) === "live", "after the retry Steer carries the fresh turn");
  flushSync(() => root.render(<></>));
}

/** Any MUI menu on Desktop takes the same Vim keys, with no local handler. */
async function checkMenuGrammar(
  root: ReturnType<typeof createRoot>,
  check: (value: unknown, message: string) => void,
  key: (key: string, extra?: KeyboardEventInit) => void,
): Promise<void> {
  const picked: string[] = [];
  let closed = 0;
  flushSync(() =>
    root.render(
      <SurfaceContext.Provider value={DESKTOP}>
        <DesktopWorkspaceProvider>
          <DesktopCommandProvider>
            <BrowserProductTheme>
              <Menu
                open
                anchorReference="anchorPosition"
                anchorPosition={{ top: 40, left: 40 }}
                onClose={() => closed++}
              >
                {["One", "Two", "Three"].map((label) => (
                  <MenuItem
                    key={label}
                    data-menu-row={label}
                    onClick={() => picked.push(label)}
                  >
                    {label}
                  </MenuItem>
                ))}
              </Menu>
            </BrowserProductTheme>
          </DesktopCommandProvider>
        </DesktopWorkspaceProvider>
      </SurfaceContext.Provider>,
    )
  );
  const row = () => document.activeElement?.getAttribute("data-menu-row");
  for (let attempt = 0; attempt < 50 && row() !== "One"; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  check(row() === "One", "A menu opens on its first row");
  key("j", { code: "KeyJ" });
  check(row() === "Two", "j moves down any Desktop menu");
  key("G", { code: "KeyG", shiftKey: true });
  check(row() === "Three", "G reaches the last menu row");
  key("k", { code: "KeyK" });
  check(row() === "Two", "k moves up any Desktop menu");
  key("l", { code: "KeyL" });
  check(picked.at(-1) === "Two", "l activates the menu row");
  key("h", { code: "KeyH" });
  check(closed === 1, "h closes the menu");
}

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
              `${checkpoint ? "checkpoint" : "steer"}:${target?.turn_id ?? "capture"}`,
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
        <SurfaceContext.Provider value={DESKTOP}>
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <Harness />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </SurfaceContext.Provider>,
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
    // Reopen only after the exit transition unmounted the list, as a person
    // does: the previous opening's options must not consume initial focus.
    await new Promise((resolve) => setTimeout(resolve, 500));
    check(
      !document.querySelector("[data-delivery-picker]"),
      "The closed picker unmounts its list",
    );
    flushSync(() => {
      setBusy(true);
      host.querySelector("button")!.click();
    });
    for (let attempt = 0; attempt < 50 && !ready(); attempt++) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(ready(), "A reopened picker focuses its default row");
    key("Enter", { altKey: true });
    check(
      !calls.some((call) => call.startsWith("interrupt")),
      "Alt Enter cannot interrupt",
    );
    // FOCUS.md "Menus": the Vim list grammar, by physical key.
    const focusedCommand = () =>
      document.activeElement?.getAttribute("data-delivery-command");
    key("j", { code: "KeyJ" });
    check(focusedCommand() === "next", "j moves to the next delivery");
    key("Process", { code: "KeyJ", keyCode: 229 } as KeyboardEventInit);
    check(
      focusedCommand() === "front",
      "A CJK input source still moves by physical j",
    );
    key("k", { code: "KeyK" });
    check(focusedCommand() === "next", "k moves to the previous delivery");
    key("G", { code: "KeyG", shiftKey: true });
    check(focusedCommand() === "pause", "G jumps to the last delivery");
    key("g", { code: "KeyG" });
    key("g", { code: "KeyG" });
    check(focusedCommand() === "steer", "gg jumps to the first delivery");
    // MenuItem animates its fill.
    await new Promise((resolve) => setTimeout(resolve, 250));
    check(
      getComputedStyle(document.activeElement!).backgroundColor !==
        "rgba(0, 0, 0, 0)",
      "The Desktop picker paints its cursor row",
    );
    check(
      document.querySelector("[data-delivery-picker-hints]")?.textContent
        ?.includes("Move"),
      "The Desktop picker names its keys",
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
    key("l", { code: "KeyL" });
    check(calls.at(-1) === "pause", "l activates the focused delivery");
    await checkMenuGrammar(root, check, key);
    await checkLiveSteer(root, check, key);
    return "Delivery picker preserves exact target, IME/repeat exclusion, disabled focus, explicit next-turn selection and the Desktop Vim menu grammar";
  } finally {
    flushSync(() => root.unmount());
    host.remove();
    globalThis.fetch = originalFetch;
  }
}
