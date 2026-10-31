/** Background tasks (docs/background-tasks.md §7) in a real browser: compact
 *  rows, the inspector (Desktop Dialog / Mobile cover Sheet), the one-line
 *  bar, keyboard and focus, Mobile Back navigation and deep links, Stop
 *  outcomes and approval expiry. Network is a fixture; stops are captured. */
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline, Stack } from "@mui/material";
import {
  buildTaskSummaries,
  CALLS_DOCK_LEGACY_KEY,
  setCallsDockLegacy,
  subagentKey,
  type TaskSummary,
  taskHash,
} from "./backgroundTasks";
import { BackgroundTasksBarView } from "./BackgroundTasksBar";
import { BackgroundTasksInspector } from "./BackgroundTasksInspector";
import { closeBackgroundTasks, inspectorState, openBackgroundTask } from "./backgroundTaskInspectorState";
import { BrowserProductTheme } from "./browserProductTheme";
import { type CallApproval, currentCallApproval, receiveCallApproval } from "./callApproval";
import { derive } from "./derive";
import type { ManagedCallSummary } from "./managedCalls";
import type { AcpUpdate, Envelope } from "./protocol";
import { classifySurface } from "./surface/profile";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { TaskRow } from "./TaskRow";
import {
  type BackgroundTasks,
  requestTaskStop,
  setStopState,
  STOP_TIMEOUT_MS,
  useStopStates,
} from "./useBackgroundTasks";

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

async function settle(frames = 1): Promise<void> {
  for (let index = 0; index < frames; index += 1) {
    await new Promise((resolve) => requestAnimationFrame(() => setTimeout(resolve, 0)));
  }
}

async function waitFor<T>(ready: () => T | null | undefined | false, label: string): Promise<T> {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    const value = ready();
    if (value) return value;
    await settle();
  }
  throw new Error(label);
}

function snapshot(seq: number, child: string, fields: Record<string, unknown>): Envelope {
  return {
    session_id: "s",
    seq,
    kind: "update",
    update: {
      sessionUpdate: "cowboy_subagent",
      sessionId: child,
      revision: seq,
      observedSeq: seq,
      title: child === "a" ? "Explore parser" : child === "b" ? "Index docs" : "Nested check",
      prompt: "Find why the parser test is flaky.",
      toolCalls: 2,
      messages: 1,
      activity: [{ kind: "tool", id: "t1", title: "Read parser.rs", status: "completed", toolKind: "read" }],
      ...fields,
    } as AcpUpdate,
  };
}

function call(overrides: Partial<ManagedCallSummary> = {}): ManagedCallSummary {
  return {
    call_id: "call-1",
    request_id: "op-1",
    provider: "codex",
    purpose: "review",
    labels: { title: "auth fallback" },
    placement: { parent_session_id: "s", machine_id: "hawk", workspace_id: "w" },
    child_session_id: "call-child",
    state: "running",
    created_at_ms: Date.now() - 100_000,
    updated_at_ms: Date.now(),
    started_at_ms: Date.now() - 100_000,
    finished_at_ms: null,
    input_revision: null,
    has_result: false,
    runtime_machine_id: "hawk",
    provider_version: null,
    cancel_requested: false,
    error: null,
    ...overrides,
  };
}

function backgroundOf(
  timeline: Envelope[],
  calls: ManagedCallSummary[] = [],
  options: { approval?: CallApproval | null; unreachable?: string[]; stopping?: string[] } = {},
): BackgroundTasks {
  const items = derive(timeline);
  const subagents = items.flatMap((item) =>
    item.kind === "subagent"
      ? [{ subagent: item.subagent, anchorSeq: Number(item.key), reconciledBy: "bootstrap" as const, reconciledAt: Date.now() }]
      : []
  );
  const tasks = buildTaskSummaries({
    subagents,
    calls,
    approval: options.approval ?? null,
    pending: [],
    unreachableMachines: new Set(options.unreachable ?? []),
    stopping: new Set(options.stopping ?? []),
  });
  return {
    items,
    tasks,
    byKey: new Map(tasks.map((task) => [task.key, task])),
    calls,
    callsObservedAt: Date.now(),
    callsError: null,
    refreshCalls: () => undefined,
    backgroundActivity: 0,
  };
}

const TIMELINE: Envelope[] = [
  snapshot(1, "a", { state: "running", cancellable: true }),
  snapshot(2, "b", { state: "idle", stopReason: "end_turn", result: "Indexed 40 pages." }),
  snapshot(3, "c", { state: "idle", stopReason: "error", error: "Tool failed", parentSessionId: "a" }),
];

function Row({ task, desktop, send }: {
  task: TaskSummary;
  desktop: boolean;
  send: (session: string, child: string) => boolean;
}): React.JSX.Element {
  const stops = useStopStates();
  return <TaskRow task={task} sessionId="s" desktop={desktop} stop={stops.get(task.key)} stopSender={send} />;
}

export async function runBackgroundTasksBrowserConformance(): Promise<string[]> {
  const passed: string[] = [];
  const sent: string[] = [];
  const send = (session: string, child: string): boolean => {
    sent.push(`${session}/${child}`);
    return true;
  };
  const originalFetch = globalThis.fetch;
  globalThis.fetch = ((input: RequestInfo | URL) => {
    const url = String(input);
    if (/\/api\/sessions\/s\/calls\/call-1$/.test(url)) {
      return Promise.resolve(Response.json({ schema: 1, call: { ...call(), request: null, result: null } }));
    }
    if (url.includes("/bootstrap")) return Promise.resolve(Response.json({ messages: [] }));
    return Promise.resolve(new Response("not found", { status: 404 }));
  }) as typeof fetch;
  const container = document.createElement("div");
  document.body.append(container);
  let root: Root = createRoot(container);
  let surface = DESKTOP;
  const render = (node: React.JSX.Element): void =>
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={surface}>
          <BrowserProductTheme>
            <CssBaseline />
            {node}
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
  const scene = (background: BackgroundTasks, desktop: boolean): void => {
    surface = desktop ? DESKTOP : MOBILE;
    render(
      <Box
        data-fixture-transcript
        sx={{ width: desktop ? 760 : 390, height: 520, overflowY: "auto", p: 1 }}
      >
        <Stack spacing={1}>
          {Array.from({ length: 12 }, (_, index) => (
            <Box key={`text-${String(index)}`} sx={{ typography: "body2" }}>
              Transcript paragraph {index}: the parent explains what it delegates.
            </Box>
          ))}
          {background.tasks.filter((task) => task.source !== "approval").map((task) => (
            <Row key={task.key} task={task} desktop={desktop} send={send} />
          ))}
          {Array.from({ length: 12 }, (_, index) => (
            <Box key={`after-${String(index)}`} sx={{ typography: "body2" }}>Later output {index}.</Box>
          ))}
        </Stack>
        <BackgroundTasksBarView sessionId="s" desktop={desktop} tasks={background.tasks} />
        <BackgroundTasksInspector sessionId="s" desktop={desktop} background={background} />
      </Box>,
    );
  };
  const inspector = (): HTMLElement | null =>
    document.querySelector<HTMLElement>("[data-background-tasks-inspector='s']");
  const press = (target: Element, key: string, init: KeyboardEventInit = {}): void => {
    target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
  };
  try {
    const background = backgroundOf(TIMELINE, [call()]);

    // 1. Rows: one per anchor, chip last, nested count, no inline expansion;
    // T14: opening and closing details changes neither the row rect nor the
    // transcript's scroll height, at 100% and at 200% text scale.
    for (const scale of ["100%", "200%"]) {
      document.documentElement.style.fontSize = scale;
      scene(background, true);
      await settle(2);
      const scroller = document.querySelector<HTMLElement>("[data-fixture-transcript]")!;
      const row = await waitFor(
        () => document.querySelector<HTMLElement>(`[data-task-row="${subagentKey("a")}"]`),
        "the subagent row renders",
      );
      check(row.lastElementChild?.querySelector("[data-task-chip]"), "the chip is the last element");
      check(row.textContent?.includes("(+1)"), "a nested child is counted");
      check(row.querySelector("[data-task-child-attention]"), "a failed nested child marks its parent");
      check(document.querySelector("[data-call-row='call-1']"), "the call anchor row renders");
      row.scrollIntoView({ block: "center" });
      const rect = row.getBoundingClientRect();
      const height = scroller.scrollHeight;
      const top = scroller.scrollTop;
      row.querySelector<HTMLElement>("[data-task-open]")!.click();
      await waitFor(inspector, "the inspector opens");
      await settle(2);
      const openRect = row.getBoundingClientRect();
      check(
        openRect.top === rect.top && openRect.height === rect.height && scroller.scrollHeight === height,
        `T14 opening details must not reflow the transcript at ${scale}`,
      );
      press(inspector()!, "Escape");
      await waitFor(() => inspector() === null, "Escape closes");
      await settle(2);
      const closedRect = row.getBoundingClientRect();
      check(
        closedRect.top === rect.top && scroller.scrollHeight === height && scroller.scrollTop === top,
        `T14 closing details must not reflow the transcript at ${scale}`,
      );
    }
    document.documentElement.style.fontSize = "";
    passed.push("T14 rows open details without reflow at 100% and 200% text scale");

    // 2. T15 Desktop keyboard: Enter on a row opens at that key; j/k and [/]
    // move a stable selection; x confirms Stop; Esc returns focus to the row.
    scene(background, true);
    await settle();
    const open = document.querySelector<HTMLElement>(`[data-task-row="${subagentKey("a")}"] [data-task-open]`)!;
    open.focus();
    open.click();
    const dialog = await waitFor(inspector, "the inspector opens from the row");
    check(inspectorState().key === subagentKey("a"), "opens at the row's key");
    check(
      document.querySelector(`[data-task-option="${subagentKey("a")}"]`)?.getAttribute("aria-selected") === "true",
      "the row's task is selected",
    );
    const order = [...document.querySelectorAll<HTMLElement>("[data-task-option]")].map((node) => node.dataset["taskOption"]);
    press(dialog, "j");
    await settle();
    check(inspectorState().key === order[order.indexOf(subagentKey("a")) + 1], "j selects the next task");
    press(dialog, "k");
    await settle();
    check(inspectorState().key === subagentKey("a"), "k selects the previous task");
    press(dialog, "]");
    await settle();
    press(dialog, "[");
    await settle();
    check(inspectorState().key === subagentKey("a"), "[ and ] step through tasks");
    check(document.querySelector("[data-task-detail-stop]"), "a working subagent offers Stop in its details");
    press(dialog, "x", { isComposing: true });
    await settle();
    check(!document.querySelector("[data-subagent-stop-confirm]"), "nothing triggers during IME composition");
    press(dialog, "x");
    await waitFor(() => document.querySelector("[data-subagent-stop-confirm]"), "x opens the Stop confirmation");
    [...document.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent?.startsWith("Keep running"))!
      .click();
    await waitFor(() => !document.querySelector("[data-subagent-stop-confirm]"), "Keep running closes it");
    check(sent.length === 0, "keeping it running sent nothing");
    press(dialog, "Escape");
    await waitFor(() => inspector() === null, "Esc closes the inspector");
    await waitFor(() => document.activeElement === open, "T15 focus returns to the originating row");
    passed.push("T15 desktop keyboard selection, stop confirmation, IME suppression and focus return");

    // 3. The bar: one line counting tasks only, opening the inspector.
    const bar = document.querySelector<HTMLElement>("[data-background-tasks-bar='s']");
    check(bar, "the bar renders while a task works");
    check(bar.textContent?.includes("◐ 2 working"), "the bar counts working tasks (subagent + call)");
    check(bar.textContent?.includes("✕ 1 failed"), "an unseen failure is counted");
    check(bar.getBoundingClientRect().height < 48, "the bar is one line");
    bar.querySelector<HTMLElement>("[data-background-tasks-open]")!.click();
    await waitFor(inspector, "the bar opens the inspector");
    check(
      [...document.querySelectorAll("[data-task-section]")].map((node) => node.getAttribute("data-task-section")).join() ===
        "needs_you,active,ended",
      "the list groups Needs you / Active / Ended",
    );
    closeBackgroundTasks();
    await waitFor(() => inspector() === null, "closing from the bar");
    scene(backgroundOf([snapshot(2, "b", { state: "idle", stopReason: "end_turn" })]), true);
    await settle();
    check(!document.querySelector("[data-background-tasks-bar='s']"), "the bar hides when nothing needs attention");
    check(
      document.querySelector("[data-background-tasks-available='true']"),
      "the history stays reachable (␣G) while the bar hides",
    );
    passed.push("the bar is one line and hides when idle");

    // 4. T9/T8 Stop outcomes: Disconnected and Timed out keep the state and
    // never claim Stopped; the source's final state resolves Stopping.
    const working = backgroundOf([snapshot(1, "a", { state: "running", cancellable: true })]);
    const task = working.byKey.get(subagentKey("a"))!;
    requestTaskStop("s", task, () => false);
    scene(working, true);
    await waitFor(
      () => document.querySelector(`[data-task-row="${subagentKey("a")}"]`)?.textContent?.includes("Disconnected"),
      "T9 a Stop that could not be sent says Disconnected",
    );
    check(document.querySelector(`[data-task-row="${subagentKey("a")}"]`)?.textContent?.includes("Working"), "state unchanged");
    setStopState(task.key, { outcome: "timed_out", message: null, at: Date.now() - STOP_TIMEOUT_MS });
    await waitFor(
      () => document.querySelector(`[data-task-row="${subagentKey("a")}"]`)?.textContent?.includes("Timed out, still running"),
      "T9 an unconfirmed Stop times out visibly",
    );
    requestTaskStop("s", task, send);
    check(sent.at(-1) === "s/a", "the Stop addresses exactly this child");
    // useBackgroundTasks feeds accepted stops into the summaries.
    scene(backgroundOf([snapshot(1, "a", { state: "running", cancellable: true })], [], { stopping: [task.key] }), true);
    await waitFor(
      () => document.querySelector(`[data-task-row="${subagentKey("a")}"] [data-task-chip]`)?.textContent === "Stopping",
      "an accepted Stop reads Stopping",
    );
    // The adapter emitted end_turn while the Stop was in flight: Done wins.
    const finished = backgroundOf([
      snapshot(1, "a", { state: "running", cancellable: true }),
      snapshot(9, "a", { state: "idle", stopReason: "end_turn", cancellable: false }),
    ]);
    setStopState(task.key, null);
    scene(finished, true);
    await settle();
    check(
      document.querySelector(`[data-task-row="${subagentKey("a")}"] [data-task-chip]`)?.textContent === "Done",
      "T8 the source's final state wins; no lingering Stopping",
    );
    passed.push("T8/T9 stop outcomes are explicit and never claim Stopped");

    // 5. T12 a call keeps its execution state while its Machine is away.
    scene(backgroundOf([], [call()], { unreachable: ["hawk"] }), true);
    await settle();
    const callRow = document.querySelector<HTMLElement>("[data-call-row='call-1']");
    check(callRow?.querySelector("[data-task-health]")?.textContent === "Reconnecting", "T12 a Reconnecting badge");
    check(callRow.querySelector("[data-task-chip]")?.textContent === "Working", "T12 no terminal claim");
    passed.push("T12 call ledger state is kept while reconnecting");

    // 6. T10 an expired approval is not actionable in the bar.
    const approval: CallApproval = {
      schema: 1,
      caller: "claude-code",
      reason: "calls_disabled",
      requests: 1,
      agents: ["codex"],
      items: [{ approval_request_id: "apr-1", agent: "codex", purpose: "review", summary: "Review" }],
      request_ids: ["apr-1"],
      prompt_revision: 1,
      ttl_ms: 15_000,
    };
    receiveCallApproval("s", approval, Date.now() - 20_000);
    scene(backgroundOf([], [], { approval: currentCallApproval("s") }), true);
    await settle();
    check(!document.querySelector("[data-background-tasks-bar='s']"), "T10 an expired prompt is not in the bar");
    receiveCallApproval("s", approval);
    scene(backgroundOf([], [], { approval: currentCallApproval("s") }), true);
    await settle();
    check(
      document.querySelector("[data-background-tasks-bar='s']")?.textContent?.includes("needs you"),
      "a live prompt is a needs-you item",
    );
    receiveCallApproval("s", null);
    passed.push("T10 expired approvals are not actionable");

    // 7. T16 Mobile: bar -> list -> detail -> conversation, one history entry
    // per level; Back walks up and returns to the transcript at its scroll.
    scene(background, false);
    await settle();
    const scroller = document.querySelector<HTMLElement>("[data-fixture-transcript]")!;
    scroller.scrollTop = 120;
    const scrollBefore = scroller.scrollTop;
    const historyBefore = history.length;
    document.querySelector<HTMLElement>("[data-background-tasks-open]")!.click();
    // The bar opens at its lead task; return to the list level first.
    await waitFor(inspector, "the Mobile inspector opens");
    history.back();
    await waitFor(
      () => document.querySelector("[data-mobile-task-level='list']"),
      "Back from the lead's details shows the list",
    );
    document.querySelector<HTMLElement>("[data-task-option='call:call-1']")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='detail']"), "a task opens its details");
    check(location.hash === taskHash("call:call-1"), "the detail level is deep-linkable");
    document.querySelector<HTMLElement>("[data-task-open-conversation]")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='conversation']"), "the call conversation opens");
    check(history.length > historyBefore, "each level is a history entry");
    history.back();
    await waitFor(() => document.querySelector("[data-mobile-task-level='detail']"), "Back returns to details");
    history.back();
    await waitFor(() => document.querySelector("[data-mobile-task-level='list']"), "Back returns to the list");
    history.back();
    await waitFor(() => inspector() === null, "Back returns to the transcript");
    check(scroller.scrollTop === scrollBefore, "T16 the transcript keeps its scroll position");
    passed.push("T16 mobile list, detail and conversation walk back level by level");

    // 8. T16 deep link: `#task=` opens that task's details.
    history.replaceState(history.state, "", taskHash(subagentKey("b")));
    root.unmount();
    root = createRoot(container);
    scene(background, false);
    await waitFor(() => document.querySelector(`[data-task-detail="${subagentKey("b")}"]`), "T16 #task= opens details");
    check(document.querySelector("[data-task-diagnostics]"), "details carry Diagnostics");
    check(
      document.querySelector("[data-diagnostic='reconciled_by']")?.textContent === "bootstrap",
      "Diagnostics name how the row was reconciled",
    );
    closeBackgroundTasks();
    history.replaceState(history.state, "", location.pathname);
    await settle();
    passed.push("T16 deep links open details with diagnostics");

    // 9. The rollback switch restores the previous Calls panel per device.
    setCallsDockLegacy(true);
    check(localStorage.getItem(CALLS_DOCK_LEGACY_KEY) === "1", "the switch is a device preference");
    setCallsDockLegacy(false);
    check(localStorage.getItem(CALLS_DOCK_LEGACY_KEY) === "0", "the switch turns off");
    openBackgroundTask("s", null, "command");
    surface = DESKTOP;
    scene(background, true);
    await waitFor(() => document.querySelector("[data-calls-dock-legacy-switch]"), "the inspector offers the switch");
    closeBackgroundTasks();
    passed.push("the Calls panel rollback switch is reachable");
  } finally {
    document.documentElement.style.fontSize = "";
    globalThis.fetch = originalFetch;
    root.unmount();
    container.remove();
  }
  return passed;
}

/** Leave the rows and bar drawn, with the inspector open at `open` (a task
 *  key, `"list"`, or nothing), for screenshots. */
export function renderBackgroundTasksPreview(desktop: boolean, open: string | null = null): void {
  if (open !== null) openBackgroundTask("s", open === "list" ? null : open, "row");
  const container = document.createElement("div");
  document.body.append(container);
  const background = backgroundOf(TIMELINE, [call(), call({
    call_id: "call-2",
    labels: { title: "design review" },
    state: "completed",
    finished_at_ms: Date.now() - 5_000,
  })]);
  createRoot(container).render(
    <SurfaceContext.Provider value={desktop ? DESKTOP : MOBILE}>
      <BrowserProductTheme>
        <CssBaseline />
        <Stack spacing={1} sx={{ width: desktop ? 760 : 390, p: 2 }}>
          {background.tasks.filter((task) => task.source !== "approval").map((task) => (
            <TaskRow key={task.key} task={task} sessionId="s" desktop={desktop} />
          ))}
          <BackgroundTasksBarView sessionId="s" desktop={desktop} tasks={background.tasks} />
        </Stack>
        <BackgroundTasksInspector sessionId="s" desktop={desktop} background={background} />
      </BrowserProductTheme>
    </SurfaceContext.Provider>,
  );
}
