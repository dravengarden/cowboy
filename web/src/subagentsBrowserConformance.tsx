/** Actual subagent rows and the approval surfaces that name a subagent,
 *  derived from a scripted timeline and driven through the DOM. A stop is
 *  captured instead of sent. Mobile and Desktop rows run under their own
 *  surface profile, so each gets its own confirmation surface. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline, Stack } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { derive, pendingPermissions, type RenderItem } from "./derive";
import { DesktopPermissionDock } from "./desktop/DesktopApprovalDock";
import { PermissionOverlay } from "./PermissionOverlay";
import type { AcpUpdate, Envelope } from "./protocol";
import { SubagentCard } from "./SubagentCard";
import { focusedSubagentStop } from "./subagents";
import { classifySurface } from "./surface/profile";
import { SurfaceContext } from "./surface/SurfaceProfile";

const DESKTOP = classifySurface({
  finePointer: true,
  hover: true,
  tabletWidth: true,
  touchCapable: false,
  desktopHost: true,
});
const MOBILE = classifySurface({
  finePointer: false,
  hover: false,
  tabletWidth: false,
  touchCapable: true,
  desktopHost: false,
});

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

function snapshot(seq: number, fields: Record<string, unknown>): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: { sessionUpdate: "cowboy_subagent", sessionId: "child", ...fields } as AcpUpdate,
  };
}

const RUNNING = {
  revision: 1,
  title: "Explore",
  description: "Explore",
  state: "running",
  cancellable: true,
  prompt: "Find why the parser test is flaky.",
  toolCalls: 2,
  messages: 1,
  activity: [
    { kind: "tool", id: "t1", title: "Read parser.rs", status: "completed", toolKind: "read" },
    { kind: "tool", id: "t2", title: "Run cargo test parser", status: "in_progress", toolKind: "execute" },
    { kind: "message", text: "The fixture races the clock." },
  ],
  omittedActivity: 3,
};

function row(timeline: Envelope[]): Extract<RenderItem, { kind: "subagent" }> {
  const item = derive(timeline).find((entry) => entry.kind === "subagent");
  check(item?.kind === "subagent", "the timeline derives one subagent row");
  return item;
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

export async function runSubagentsBrowserConformance(): Promise<string[]> {
  const passed: string[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const stops: string[] = [];
  let surface = MOBILE;
  const render = (node: React.JSX.Element): void =>
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={surface}>
          <BrowserProductTheme>
            <CssBaseline />
            <Stack sx={{ width: 420, p: 2 }}>{node}</Stack>
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
  let mounts = 0;
  // `remount` starts a fresh row; without it, an update keeps its UI state.
  const card = (timeline: Envelope[], desktop = false, remount = true): void => {
    surface = desktop ? DESKTOP : MOBILE;
    render(
      <SubagentCard
        key={remount ? ++mounts : mounts}
        subagent={row(timeline).subagent}
        sessionId="s"
        desktop={desktop}
        stop={(session, child): boolean => {
          stops.push(`${session}/${child}`);
          return true;
        }}
      />,
    );
  };
  const confirmButton = (): HTMLButtonElement | null =>
    document.querySelector<HTMLButtonElement>("[data-subagent-stop-confirm]");
  const waitFor = async (ready: () => boolean, label: string): Promise<void> => {
    for (let attempt = 0; attempt < 100 && !ready(); attempt++) await settle();
    check(ready(), label);
  };

  // 1. A working subagent is one collapsed row: who, state, what it is doing.
  card([snapshot(1, RUNNING)]);
  const element = container.querySelector<HTMLElement>('[data-subagent-row="child"]');
  check(element, "the row renders");
  check(element.dataset["subagentState"] === "running", "the row carries its state");
  check(element.textContent?.includes("Explore"), "the row names the subagent");
  check(element.textContent?.includes("Working"), "the row states that it works");
  check(element.textContent?.includes("2 tool calls · 1 message"), "the row counts its activity");
  check(
    container.querySelector("[data-subagent-preview]")?.textContent ===
      "The fixture races the clock.",
    "the preview is the latest activity",
  );
  check(!container.querySelector("[data-subagent-activity]"), "activity starts collapsed");
  passed.push("a working subagent is one collapsed row");

  // 2. Its own activity opens in place, bounded and in order.
  const toggle = container.querySelector<HTMLButtonElement>(
    '[data-subagent-row="child"] [data-desktop-item-action="default"]',
  );
  check(toggle?.getAttribute("aria-expanded") === "false", "the toggle is the row's default action");
  toggle.click();
  await settle();
  const activity = container.querySelector<HTMLElement>("[data-subagent-activity]");
  check(activity, "activity expands");
  check(toggle.getAttribute("aria-expanded") === "true", "the toggle reports the expansion");
  const entries = [...activity.querySelectorAll<HTMLElement>("[data-subagent-activity-entry]")];
  check(
    entries.map((entry) => entry.dataset["subagentActivityEntry"]).join(",") === "tool,tool,message",
    "activity keeps its order",
  );
  check(activity.textContent?.includes("Find why the parser test is flaky."), "the delegation prompt is shown");
  check(activity.textContent?.includes("3 earlier steps not shown"), "omitted steps are counted");
  passed.push("activity expands in place");

  // 3. On Mobile, Stop asks first; keeping it running sends nothing, and the
  // confirmed Stop addresses this subagent only, once.
  const stop = container.querySelector<HTMLButtonElement>("[data-subagent-stop]");
  check(stop && !stop.disabled, "a cancellable subagent offers Stop");
  stop.click();
  await waitFor(() => confirmButton() !== null, "Stop opens a confirmation");
  check(stops.length === 0, "nothing is sent before the confirmation");
  check(document.body.textContent?.includes("Stop subagent Explore?"), "the confirmation names the subagent");
  const keep = [...document.querySelectorAll<HTMLButtonElement>("button")]
    .find((button) => button.textContent?.startsWith("Keep running"));
  check(keep, "the confirmation can keep the subagent running");
  keep.click();
  await waitFor(() => confirmButton() === null, "Keep running closes the confirmation");
  check(stops.length === 0, "keeping it running sends nothing");
  stop.click();
  await waitFor(() => confirmButton() !== null, "Stop asks again");
  confirmButton()?.click();
  await waitFor(() => confirmButton() === null, "confirming closes the confirmation");
  check(stops.join() === "s/child", "the confirmed Stop sends exactly this subagent");
  check(
    container.querySelector<HTMLButtonElement>("[data-subagent-stop]")?.disabled,
    "Stop waits for the subagent to report",
  );
  passed.push("mobile stop confirms and addresses one subagent");

  // 4. On Desktop the row is a Conversation widget: focusing it gives the
  // Stop key a target and draws its hint; the key's command confirms too.
  stops.length = 0;
  card([snapshot(1, { ...RUNNING, model: "Haiku 4.5" })], true);
  check(
    container.querySelector('[data-subagent-row="child"]')?.textContent?.includes("Haiku 4.5"),
    "the row names the subagent's model",
  );
  const widget = container.querySelector<HTMLButtonElement>(
    '[data-subagent-row="child"] [data-desktop-widget-toggle="subagent"]',
  );
  check(widget, "the Desktop row is a selectable Conversation widget");
  check(focusedSubagentStop() === null, "no row has focus, so the key has no target");
  check(
    container.querySelector("[data-subagent-stop]")?.textContent?.trim() === "Stop",
    "the Stop key is drawn only on the focused row",
  );
  widget.focus();
  await settle();
  const target = focusedSubagentStop();
  check(target && target.matches("[data-subagent-stop]"), "the focused row's Stop is the key's target");
  check(target.textContent?.includes("X"), "the focused row draws its Stop key");
  target.click();
  await waitFor(() => confirmButton() !== null, "the Desktop Stop confirms in a dialog");
  check(stops.length === 0, "nothing is sent before the Desktop confirmation");
  confirmButton()?.click();
  await waitFor(() => confirmButton() === null, "the Desktop confirmation closes");
  check(stops.join() === "s/child", "the Desktop Stop sends exactly this subagent");
  passed.push("desktop stop is a focused-row key with a confirmation");

  // 5. A newer snapshot settles it: no Stop, the result is its outcome.
  card([snapshot(1, RUNNING)], true);
  card([
    snapshot(1, RUNNING),
    snapshot(2, {
      ...RUNNING,
      revision: 2,
      state: "idle",
      stopReason: "end_turn",
      cancellable: false,
      result: "The fixture used wall-clock time; use the mock clock.",
    }),
  ], true, false);
  check(!container.querySelector("[data-subagent-stop]"), "an ended subagent has no Stop");
  check(container.querySelector('[data-subagent-row="child"]')?.textContent?.includes("Done"), "it reads Done");
  check(
    container.querySelector("[data-subagent-preview]")?.textContent?.startsWith("The fixture used wall-clock"),
    "its result is the preview",
  );
  container.querySelector<HTMLButtonElement>('[data-desktop-item-action="default"]')?.click();
  await settle();
  check(
    container.querySelector("[data-subagent-activity]")?.textContent?.includes("use the mock clock"),
    "the full result is in its activity",
  );
  passed.push("a newer snapshot settles the row");

  // 6. A subagent that stopped is not presented as done; one that stopped
  // reporting claims neither success nor failure.
  card([snapshot(1, { ...RUNNING, state: "idle", stopReason: "cancelled", cancellable: false })]);
  check(container.querySelector('[data-subagent-row="child"]')?.textContent?.includes("Stopped"), "a cancelled subagent reads Stopped");
  card([snapshot(1, { ...RUNNING, reporting: false })]);
  const quiet = container.querySelector<HTMLElement>('[data-subagent-row="child"]');
  check(quiet?.dataset["subagentReporting"] === "false", "the row marks that it stopped reporting");
  check(quiet.textContent?.includes("No longer reporting"), "a silent subagent reads No longer reporting");
  check(
    !/Working|Done|Failed/.test(quiet.textContent ?? ""),
    "a silent subagent is neither working nor finished",
  );
  card([
    snapshot(1, RUNNING),
    { session_id: "s", seq: 2, kind: "lifecycle", status: "starting", detail: null },
  ]);
  check(
    container.querySelector('[data-subagent-row="child"]')?.textContent?.includes("No longer reporting"),
    "a new worker that did not announce it leaves it not reporting",
  );
  check(!container.querySelector("[data-subagent-stop]"), "a subagent nobody reports on cannot be stopped");
  passed.push("stopped and silent subagents are told apart");

  // 7. Both approval surfaces name the subagent that asks.
  const [request] = pendingPermissions([
    {
      session_id: "s",
      seq: 1,
      kind: "permission_request",
      request_id: "r-1",
      tool_call: {
        toolCallId: "t3",
        title: "cargo test parser",
        _cowboy: { subagent: { sessionId: "child", title: "Explore" } },
      },
      options: [
        { optionId: "allow", name: "Allow", kind: "allow_once" },
        { optionId: "reject", name: "Reject", kind: "reject_once" },
      ],
    },
  ]);
  check(request?.subagent === "Explore", "the pending request names its subagent");
  surface = DESKTOP;
  render(<DesktopPermissionDock item={request} waiting={1} sessionId="s" />);
  check(container.textContent?.includes("Asked by subagent Explore"), "the Desktop dock names the subagent");
  surface = MOBILE;
  render(<PermissionOverlay item={request} sessionId="s" />);
  check(
    container.querySelector("[data-permission-subagent]")?.textContent === "Explore",
    "the Mobile overlay names the subagent",
  );
  passed.push("approval surfaces name the subagent");

  root.unmount();
  container.remove();
  return passed;
}

/** Leave a working and a finished subagent drawn for a visual review. */
export function renderSubagentsPreview(): void {
  const container = document.createElement("div");
  document.body.append(container);
  const finished = row([snapshot(1, {
    ...RUNNING,
    title: "Reviewer",
    state: "idle",
    stopReason: "end_turn",
    cancellable: false,
    result: "No blocking findings.",
  })]).subagent;
  const quiet = row([snapshot(1, { ...RUNNING, title: "Indexer", reporting: false, model: "Haiku 4.5" })])
    .subagent;
  createRoot(container).render(
    <SurfaceContext.Provider value={DESKTOP}>
      <BrowserProductTheme>
        <CssBaseline />
        <Stack spacing={1} sx={{ width: 520, p: 2 }}>
          <SubagentCard subagent={row([snapshot(1, RUNNING)]).subagent} sessionId="s" desktop={false} stop={() => true} />
          <SubagentCard subagent={finished} sessionId="s" desktop />
          <SubagentCard subagent={quiet} sessionId="s" desktop stop={() => true} />
        </Stack>
      </BrowserProductTheme>
    </SurfaceContext.Provider>,
  );
}
