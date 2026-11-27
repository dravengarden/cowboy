/** Background tasks (docs/background-tasks.md §7) in a real browser: compact
 *  rows, the inspector (Desktop Dialog / Mobile cover Sheet), the one-line
 *  bar, keyboard and focus, Mobile Back navigation and deep links, Stop
 *  outcomes and approval expiry. Network is a fixture; stops are captured. */
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline, Stack } from "@mui/material";
import { MobileSheetContextDock } from "@cowboy/app-shell";
import {
  buildTaskSummaries,
  CALLS_DOCK_LEGACY_KEY,
  setCallsDockLegacy,
  subagentKey,
  type TaskSummary,
  taskHash,
  setAcknowledgementTransport,
} from "./backgroundTasks";
import { BackgroundTasksBarView } from "./BackgroundTasksBar";
import { BackgroundTasksInspector } from "./BackgroundTasksInspector";
import { type ObservedPage, setSubagentTranscriptTransport } from "./observedTranscript";
import { closeBackgroundTasks, inspectorState, openBackgroundTask } from "./backgroundTaskInspectorState";
import { BrowserProductTheme } from "./browserProductTheme";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { DesktopCommandHost } from "./desktop/commands/DesktopCommandHost";
import { DesktopWorkspace } from "./desktop/DesktopWorkspace";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { desktopWorkView, requestDesktopWorkView } from "./desktop/review/desktopWorkView";
import { getDraft, setDraft } from "./draftStore";
import { DesktopTasksView } from "./tasks/DesktopTasksView";
import { type CallApproval, currentCallApproval, receiveCallApproval } from "./callApproval";
import { derive } from "./derive";
import { ProviderQuotaStatusRow } from "./ProviderQuotaStatus";
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
    configuration: { model: "gpt-6.1-sol", reasoning_effort: "max", permission_mode: "read-only" },
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
    unresolvedCalls: new Set(),
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

export async function runBackgroundTasksBrowserConformance(observe?: () => Promise<void>): Promise<string[]> {
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
            {surface.kind === "desktop"
              ? (
                <DesktopWorkspaceProvider>
                  <DesktopCommandProvider>{node}</DesktopCommandProvider>
                </DesktopWorkspaceProvider>
              )
              : node}
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
  const scene = (background: BackgroundTasks, desktop: boolean): void => {
    surface = desktop ? DESKTOP : MOBILE;
    render(
      <Box
        data-fixture-transcript
        sx={{ width: desktop ? 760 : 390, maxWidth: "100%", height: 520, overflowY: "auto", p: 1 }}
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
    const paper = dialog.closest<HTMLElement>(".MuiDialog-paper")!;
    const paperStyle = getComputedStyle(paper);
    const paperRect = paper.getBoundingClientRect();
    check(
      paperStyle.maxWidth === "none" && paperStyle.maxHeight === "none" &&
        paperRect.width >= innerWidth - 32 && paperRect.height >= innerHeight - 32,
      "desktop background-tasks inspector claims the available viewport without artificial caps",
    );
    check(inspectorState().key === subagentKey("a"), "opens at the row's key");
    check(
      document.querySelector(`[data-task-option="${subagentKey("a")}"]`)?.getAttribute("aria-selected") === "true",
      "the row's task is selected",
    );
    const order = [...document.querySelectorAll<HTMLElement>("[data-task-option]")].map((node) => node.dataset["taskOption"]);
    press(document.activeElement ?? dialog, "j");
    await settle();
    check(inspectorState().key === order[order.indexOf(subagentKey("a")) + 1], "j selects the next task under DesktopCommandProvider");
    press(document.activeElement ?? dialog, "k");
    await settle();
    check(inspectorState().key === subagentKey("a"), "k selects the previous task under DesktopCommandProvider");
    // Focus dropping to document.body (e.g. an unmounted task button) must not kill shortcuts.
    (document.activeElement as HTMLElement | null)?.blur?.();
    press(document.body, "j", { code: "KeyJ" });
    await settle();
    check(inspectorState().key === order[order.indexOf(subagentKey("a")) + 1], "j still works when focus drops to document.body");
    // Idle CJK IME reporting Process/229 or full-width brackets must still drive shortcuts.
    press(document.body, "Process", { code: "KeyK", keyCode: 229 });
    await settle();
    check(inspectorState().key === subagentKey("a"), "idle CJK Process/229 k still steps backward");
    press(document.body, "】", { code: "BracketRight", keyCode: 229 });
    await settle();
    check(inspectorState().key === order[order.indexOf(subagentKey("a")) + 1], "idle CJK 】 steps forward");
    press(document.body, "【", { code: "BracketLeft", keyCode: 229 });
    await settle();
    check(inspectorState().key === subagentKey("a"), "idle CJK 【 steps backward");
    press(document.activeElement ?? dialog, "ArrowDown");
    await settle();
    check(inspectorState().key === order[order.indexOf(subagentKey("a")) + 1], "ArrowDown selects the next task");
    press(document.activeElement ?? dialog, "ArrowUp");
    await settle();
    check(inspectorState().key === subagentKey("a"), "ArrowUp selects the previous task");
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
    // Escape inside the Stop confirmation closes only the confirmation, never the inspector beneath.
    press(document.activeElement ?? document.body, "Escape", { code: "Escape" });
    await waitFor(() => !document.querySelector("[data-subagent-stop-confirm]"), "Esc closes only the Stop confirmation");
    check(inspector() !== null, "Esc inside Stop confirmation keeps the inspector open");
    check(sent.length === 0, "dismissing Stop confirmation sent nothing");
    press(document.body, "Process", { code: "Escape", keyCode: 229 });
    await waitFor(() => inspector() === null, "Esc closes the inspector");
    await waitFor(() => document.activeElement === open, "T15 focus returns to the originating row");
    passed.push("T15 desktop keyboard selection, stop confirmation, IME suppression and focus return");

    // 3. The bar: one line counting tasks only, opening the inspector.
    const bar = document.querySelector<HTMLElement>("[data-background-tasks-bar='s']");
    check(bar, "the bar renders while a task works");
    check(bar.textContent?.includes("◐ 2 working"), "the bar counts working tasks (subagent + call)");
    check(bar.textContent?.includes("✕ 1 failed"), "an unseen failure is counted");
    check(bar.getBoundingClientRect().height < 48, "the bar is one line");
    check(bar.getBoundingClientRect().height < 32, "desktop background status uses a compact row");
    bar.style.width = "220px";
    await settle();
    const opener = bar.querySelector<HTMLElement>("[data-background-tasks-open]")!;
    check(opener.scrollWidth <= opener.clientWidth + 1, "status counts do not overflow a narrow pane");
    check(
      opener.querySelector("svg")!.getBoundingClientRect().right <= opener.getBoundingClientRect().right,
      "the inspector entry stays inside a narrow pane",
    );
    bar.style.width = "";
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
    scene(backgroundOf([], [], { approval: currentCallApproval("s") }), false);
    openBackgroundTask("s", null, "bar");
    await waitFor(() => document.querySelector("[data-task-review-request]"), "T10 actionable mobile triage card offers explicit Review request button");
    closeBackgroundTasks();
    receiveCallApproval("s", null);
    passed.push("T10 expired approvals are not actionable");

    // 7. T16 Mobile: 1-tap Triage Peek -> same-host Reader -> 1-tap sibling switcher (replaceState)
    //    -> Info with Diagnostics -> Quote to Prompt -> Back unwinds cleanly preserving scroll.
    scene(background, false);
    await settle();
    const scroller = document.querySelector<HTMLElement>("[data-fixture-transcript]")!;
    scroller.scrollTop = 120;
    const scrollBefore = scroller.scrollTop;
    const historyBefore = history.length;
    document.querySelector<HTMLElement>("[data-background-tasks-open]")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='peek']"), "T16 1 tap opens Triage Peek directly");
    {
      const sheetPaper = document.querySelector<HTMLElement>("[data-detent-sheet='true']")!;
      check(sheetPaper.getBoundingClientRect().height <= innerHeight * 0.62, "T16 Triage Peek caps height so main chat remains visible above");
      check(Boolean(document.querySelector("[data-mobile-tasks-peek-footer]")), "T16 Triage Peek anchors controls to its visible bottom");
      check(Boolean(document.querySelector("[data-task-tree-row='subagent:c'][data-task-depth='1']")), "T16 real parentKey tree indents nested child under parent");
    }
    document.querySelector<HTMLElement>("[data-task-option='call:call-1']")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='reader']"), "T16 tapping a card expands same sheet into Reader");
    check(location.hash === taskHash("call:call-1"), "T16 reader level is deep-linkable");
    check(document.querySelector("[data-call-transcript]"), "T16 active call opens live transcript first");
    check(!document.querySelector("[data-task-diagnostics]"), "T16 static diagnostics stay folded behind Info");
    {
      const dock = document.querySelector<HTMLElement>("[data-mobile-tasks-dock]")!;
      const footer = dock.closest<HTMLElement>("[data-detent-sheet-footer]")!;
      const dialog = dock.closest<HTMLElement>('[role="dialog"]')!;
      const measured = Number.parseFloat(getComputedStyle(dialog).getPropertyValue("--cowboy-sheet-dock-clearance"));
      check(Math.abs(measured - footer.getBoundingClientRect().height) < 2, "T16 dock clearance matches measured footer height");
      check(Boolean(document.querySelector("[data-mobile-worker-strip]")), "T16 bottom worker switcher strip renders for sibling tasks");
    }
    await observe?.();
    const historyAtReader = history.length;
    document.querySelector<HTMLElement>(`[data-mobile-worker-pill="${subagentKey("b")}"]`)!.click();
    await waitFor(() => document.querySelector(`[data-task-reader="${subagentKey("b")}"]`), "T16 1-tap sibling pill switches worker directly");
    check(history.length === historyAtReader, "T16 sibling switch uses replaceState without piling Back history");
    check(document.querySelector(`[data-task-reader-mode="result"]`), "T16 ended worker leads with Result summary");
    document.querySelector<HTMLElement>("[data-mobile-tasks-info]")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='info']"), "T16 Info opens execution details & diagnostics");
    check(Boolean(document.querySelector("[data-task-diagnostics]")), "T16 Info carries Diagnostics");
    // Data refresh while reading Info must NOT eject the reader back to Result!
    scene({ ...background, byKey: new Map(background.byKey) }, false);
    await settle();
    check(Boolean(document.querySelector("[data-mobile-task-level='info']")), "T16 Info stays open across background data refreshes");
    check(history.length > historyBefore, "T16 distinct levels use browser history");
    // Closing Info with its own button unwinds history synchronously with popstate.
    document.querySelector<HTMLElement>("[data-mobile-tasks-info]")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='reader']"), "T16 Info button returns cleanly to Reader");
    // Peek -> Reader -> All unwinds back to the existing Peek entry so Back closes sheet cleanly.
    document.querySelector<HTMLElement>("[data-mobile-tasks-all]")!.click();
    await waitFor(() => document.querySelector("[data-mobile-task-level='peek']"), "T16 All returns to Triage Peek");
    history.back();
    await waitFor(() => inspector() === null, "T16 Back from Peek after All returns directly to main transcript");
    check(scroller.scrollTop === scrollBefore, "T16 transcript keeps its scroll position");
    setDraft("s", { text: "", attachments: [] });
    openBackgroundTask("s", subagentKey("b"), "row");
    await waitFor(() => document.querySelector("[data-mobile-tasks-quote]"), "T16 Quote action is reachable in bottom dock");
    document.querySelector<HTMLElement>("[data-mobile-tasks-quote]")!.click();
    await waitFor(() => inspector() === null, "T16 Quote closes task sheet back to chat");
    await settle(3);
    check(getDraft("s").text.includes("Index docs"), `T16 Quote appends worker reference to parent draft (got: ${JSON.stringify(getDraft("s").text)})`);
    setDraft("s", { text: "", attachments: [] });
    passed.push("T16 mobile triage peek, 1-tap sibling switcher, info, quote and Back navigation");

    // 8. T16 deep link: `#task=` opens that task's details.
    history.replaceState(history.state, "", taskHash(subagentKey("b")));
    root.unmount();
    root = createRoot(container);
    scene(background, false);
    await waitFor(() => document.querySelector(`[data-task-detail="${subagentKey("b")}"]`), "T16 #task= opens result-first reader");
    document.querySelector<HTMLElement>("[data-mobile-tasks-info]")!.click();
    await waitFor(() => document.querySelector("[data-task-diagnostics]"), "Info carries Diagnostics");
    check(
      document.querySelector("[data-diagnostic='reconciled_by']")?.textContent === "bootstrap",
      "Diagnostics name how the row was reconciled",
    );
    // iOS can deliver history traversal after the close has rendered. Keep
    // the task hash in place until then: it must not reopen the sheet.
    const originalGo = history.go;
    let pendingBack = 0;
    history.go = (delta = 0): void => { pendingBack = delta; };
    try {
      document.querySelector<HTMLElement>("button[aria-label='Close']")!.click();
      await settle();
      scene({ ...background, byKey: new Map(background.byKey) }, false);
      await settle();
      check(inspector() === null, "Close stays closed while the old task hash awaits history traversal");
    } finally {
      history.go = originalGo;
    }
    if (pendingBack) {
      history.go(pendingBack);
      await settle();
    }
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

    // The quota notice is a peer of the bar: same inset and row height, one
    // line even with every limit named, and no surface of its own.
    for (const desktop of [true, false]) {
      surface = desktop ? DESKTOP : MOBILE;
      render(
        <Box sx={{ width: desktop ? 760 : 320 }}>
          <ProviderQuotaStatusRow
            displayName="Claude · Mink"
            details="5h resets 23:29 · 7d resets Nov 3 23:29"
            activeTurn
            desktop={desktop}
            refresh={(): Promise<void> => Promise.resolve()}
          />
          <BackgroundTasksBarView sessionId="s" desktop={desktop} tasks={[]} backgroundActivity={1} />
        </Box>,
      );
      await settle(2);
      const row = await waitFor(
        () => document.querySelector<HTMLElement>("[data-provider-quota-status]"),
        "quota row",
      );
      const bar = await waitFor(
        () => document.querySelector<HTMLElement>("[data-background-tasks-open]"),
        "activity bar",
      );
      const icon = row.querySelector("svg");
      const activity = bar.querySelector<HTMLElement>("[data-bar-part='activity']");
      const text = row.querySelector<HTMLElement>("span");
      const label = desktop ? "Desktop" : "Mobile";
      check(icon && activity && text, `${label} quota row parts`);
      check(
        Math.abs(icon.getBoundingClientRect().left - activity.getBoundingClientRect().left) < 1,
        `${label} quota row shares the bar inset`,
      );
      check(
        Math.abs(row.getBoundingClientRect().height - bar.getBoundingClientRect().height) < 2,
        `${label} quota row matches the bar height`,
      );
      check(text.scrollWidth >= text.clientWidth && text.getClientRects().length === 1, `${label} quota row is one line`);
      check(getComputedStyle(row).backgroundColor === "rgba(0, 0, 0, 0)", `${label} quota row has no fill`);
    }
    passed.push("the quota notice is one quiet line aligned with the bar");

    // 10. T11 an MCP-started call links to the tool card that started it.
    const opened: string[] = [];
    const withTool = backgroundOf([
      {
        session_id: "s",
        seq: 1,
        kind: "update",
        update: {
          sessionUpdate: "tool_call",
          toolCallId: "mcp-1",
          title: "cowboy_execute",
          status: "completed",
          rawInput: { action: "review.start", operation: "op-1", arguments: {} },
        } as AcpUpdate,
      },
    ], [call()]);
    openBackgroundTask("s", "call:call-1", "row");
    surface = DESKTOP;
    render(
      <BackgroundTasksInspector
        sessionId="s"
        desktop
        background={withTool}
        onOpenTool={(key): void => {
          opened.push(key);
        }}
      />,
    );
    const infoBtn = await waitFor(
      () => document.querySelector<HTMLButtonElement>("[data-task-info-toggle]"),
      "Info toggle opens call execution details on demand",
    );
    infoBtn.click();
    const origin = await waitFor(
      () => document.querySelector<HTMLButtonElement>("[data-task-open-origin]"),
      "an MCP-started call links to its tool card inside Info",
    );
    origin.click();
    await waitFor(() => inspector() === null, "following the link closes the inspector");
    check(opened.join() === "1", "the tool inspector opens that card");
    openBackgroundTask("s", "call:call-1", "row");
    render(<BackgroundTasksInspector sessionId="s" desktop background={backgroundOf([], [call({ request_id: "cli-7" })])} onOpenTool={() => undefined} />);
    await waitFor(inspector, "the inspector opens");
    document.querySelector<HTMLButtonElement>("[data-task-info-toggle]")!.click();
    await waitFor(() => document.querySelector("[data-task-info-panel]"), "Info panel opens for shell call");
    check(!document.querySelector("[data-task-open-origin]"), "a shell-started call has no structured link");
    closeBackgroundTasks();
    passed.push("T11 only structured MCP identity links a call to its tool card");

    // T19: a subagent with an observed transcript reads like a Call — the
    // transcript beside its details on Desktop, first on a phone — and one
    // with nothing observed keeps the bounded snapshot.
    setSubagentTranscriptTransport((_session, child, cursor) =>
      Promise.resolve(
        child !== "b" || cursor.after !== undefined || cursor.before !== undefined
          ? { events: [], more_before: false }
          : {
            events: [
              { seq: 1, ts_ms: 1, update: { sessionUpdate: "tool_call", toolCallId: "t1", title: "Read parser.rs", kind: "read", status: "completed" } },
              { seq: 2, ts_ms: 2, update: { sessionUpdate: "agent_message_chunk", messageId: "m", content: { type: "text", text: "The flaky test races the parser cache." } } },
            ],
            more_before: false,
          },
      )
    );
    for (const desktop of [true, false]) {
      surface = desktop ? DESKTOP : MOBILE;
      openBackgroundTask("s", subagentKey("b"), "row");
      render(<BackgroundTasksInspector sessionId="s" desktop={desktop} background={backgroundOf(TIMELINE)} />);
      const openTranscript = await waitFor(
        () => document.querySelector<HTMLButtonElement>("[data-task-open-transcript]"),
        `${desktop ? "Desktop" : "Mobile"} enables Full transcript without hijacking Result summary`,
      );
      check(
        document.querySelector('[data-task-reader-mode="result"]'),
        `${desktop ? "Desktop" : "Mobile"} keeps ended subagent on Result summary until opened`,
      );
      openTranscript.click();
      const transcript = await waitFor(
        () => document.querySelector<HTMLElement>("[data-subagent-transcript='b']"),
        `${desktop ? "Desktop" : "Mobile"} shows the observed transcript on demand`,
      );
      await waitFor(
        () => transcript.textContent?.includes("The flaky test races the parser cache.") ? true : null,
        "the child's own message renders",
      );
      check(transcript.textContent?.includes("Find why the parser test is flaky."), "the delegation prompt opens it");
      closeBackgroundTasks();
      await settle();
    }
    surface = DESKTOP;
    openBackgroundTask("s", subagentKey("c"), "row");
    render(<BackgroundTasksInspector sessionId="s" desktop background={backgroundOf(TIMELINE)} />);
    await waitFor(() => document.querySelector("[data-subagent-activity-entry]"), "an unobserved subagent keeps its snapshot");
    await settle(4);
    check(!document.querySelector("[data-subagent-transcript]"), "nothing observed shows no empty transcript");
    closeBackgroundTasks();
    setSubagentTranscriptTransport(null);
    passed.push("T19 an observed subagent reads like a Call; an unobserved one keeps its snapshot");

    // A notification's link opens details; only the explicit action acknowledges.
    const acknowledged: string[] = [];
    setAcknowledgementTransport((_, ack) => {
      acknowledged.push(ack.task_key);
      return Promise.resolve(true);
    });
    openBackgroundTask("s", subagentKey("c"), "link");
    render(<BackgroundTasksInspector sessionId="s" desktop background={backgroundOf(TIMELINE)} />);
    const markSeen = await waitFor(
      () => document.querySelector<HTMLButtonElement>("[data-task-mark-seen]"),
      "a link to an unseen failure offers Mark as seen",
    );
    await settle(2);
    check(acknowledged.length === 0, "opening through a link acknowledges nothing");
    markSeen.click();
    await waitFor(() => acknowledged.length === 1 ? true : null, "Mark as seen acknowledges");
    check(acknowledged[0] === subagentKey("c"), "the acknowledgement names the task");
    setAcknowledgementTransport(null);
    closeBackgroundTasks();
    passed.push("A notification link leaves a failure unseen until Mark as seen");

    // 14. Desktop Work Pane Tasks tab: non-modal alongside Prompt, width-responsive
    //     compact (<900px) vs split (>=900px), Quote to Prompt, Esc stays in Tasks.
    surface = DESKTOP;
    setDraft("s", { text: "", attachments: [] });
    render(
      <Box sx={{ width: 1280, height: 760, display: "flex" }}>
        <DesktopCommandHost onNewSession={() => undefined} onOpenSettings={() => undefined} />
        <DesktopWorkspace
          promptWidth={440}
          resizing={false}
          onResizeStart={() => undefined}
          sessionId="s"
          projection="history"
          onProjectionChange={() => undefined}
          prompt={<Box data-fixture-prompt-visible sx={{ p: 2 }}>Parent Prompt stays visible</Box>}
          conversation={<Box data-fixture-conversation>Main Conversation</Box>}
        />
      </Box>,
    );
    await settle(2);
    requestDesktopWorkView("tasks");
    await waitFor(() => desktopWorkView() === "tasks" ? true : null, "Desktop work view enters Tasks");
    setCallsDockLegacy(true);
    await waitFor(() => desktopWorkView() === "conversation" ? true : null, "Enabling legacy rollback exits phantom Tasks view");
    setCallsDockLegacy(false);

    const endedBg = backgroundOf([snapshot(2, "b", { state: "idle", stopReason: "end_turn", result: "Found stale cache." })]);
    render(
      <Box data-desktop-pane="conversation" sx={{ width: 560, height: 600, display: "flex", flexDirection: "column" }}>
        <DesktopTasksView sessionId="s" active providedBackground={background} />
      </Box>,
    );
    await waitFor(
      () => document.querySelector("[data-desktop-tasks-layout='compact'] [data-desktop-tasks-strip]"),
      "Desktop Tasks uses compact full-width reader + switcher strip under 900px",
    );
    document.querySelector<HTMLButtonElement>("[data-task-quote-prompt]")!.click();
    check(getDraft("s").text.includes("Regarding background task"), "Desktop Quote to Prompt appends reference to parent draft");
    render(
      <Box data-desktop-pane="conversation" sx={{ width: 960, height: 600, display: "flex", flexDirection: "column" }}>
        <DesktopTasksView sessionId="s" active providedBackground={endedBg} />
      </Box>,
    );
    await waitFor(
      () => document.querySelector("[data-desktop-tasks-layout='split'] [data-desktop-region='tasks.roster']"),
      "Desktop Tasks splits 236px roster beside reader at >= 900px Work Pane width",
    );
    check(
      document.querySelector("[data-task-reader-mode='result']") !== null,
      "Ended task opens Result-First inside Desktop Tasks view",
    );
    const desktopInfoBtn = document.querySelector<HTMLButtonElement>("[data-task-info-toggle]")!;
    desktopInfoBtn.focus();
    desktopInfoBtn.click();
    await waitFor(() => document.querySelector("[data-task-reader-mode='info']"), "Clicking Info opens task info");
    desktopInfoBtn.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await waitFor(() => document.querySelector("[data-task-reader-mode='result']"), "Escape closes clicked Info back to Result");
    const hideChip = document.querySelector<HTMLElement>("[data-task-hide-ended]")!;
    hideChip.focus();
    hideChip.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    hideChip.dispatchEvent(new KeyboardEvent("keyup", { key: "Enter", bubbles: true, cancelable: true }));
    await waitFor(
      () => document.querySelector("[data-task-hide-ended]")?.textContent?.includes("Showing active") ? true : null,
      "Roster Enter preserves native activation on non-option controls",
    );
    check(document.activeElement === hideChip, "Roster Enter on Hide ended does not move focus away to reader");
    requestDesktopWorkView("conversation");
    check(desktopWorkView() === "conversation", "Work view switches cleanly back to Conversation");
    setDraft("s", { text: "", attachments: [] });
    passed.push("Desktop Work Pane Tasks tab adapts to pane width and quotes to Prompt");
    const activated: string[] = [];
    render(<Box sx={{ width: 320 }}><MobileSheetContextDock
      context={{ title: "Reader", facts: [{ label: "Model", value: "A very long recorded model name that remains available by horizontal scrolling" }] }}
      onClose={() => activated.push("close")}
      actions={["Latest", "Open session"].map(label => ({ label, icon: <span>↗</span>, onActivate: () => activated.push(label) }))}
    /></Box>);
    const rail = document.querySelector<HTMLElement>("[data-context-facts]")!;
    check(rail.scrollWidth <= rail.clientWidth, "Long facts wrap inside the information surface");
    check(document.querySelector<HTMLElement>("[data-sheet-context-dock]")!.scrollWidth <= 320, "Information and actions do not widen the dock");
    document.querySelectorAll<HTMLButtonElement>("[data-context-actions] button").forEach(button => button.click());
    check(activated.join(",") === "Latest,Open session", "Multiple contextual actions remain directly available");
    render(<MobileSheetContextDock context={{ title: "Reader" }} onClose={() => {}} />);
    check(!document.querySelector("[data-context-facts], [data-context-actions]"), "Empty supporting rails take no space");
  } finally {
    document.documentElement.style.fontSize = "";
    globalThis.fetch = originalFetch;
    root.unmount();
    container.remove();
  }
  return passed;
}

const PREVIEW_TRANSCRIPT = {
  more_before: false,
  events: [
    { seq: 1, ts_ms: 1, update: { sessionUpdate: "agent_thought_chunk", content: { type: "text", text: "Start from the failing test, then the parser cache it touches." } } },
    { seq: 2, ts_ms: 2, update: { sessionUpdate: "tool_call", toolCallId: "t1", title: "rg -n \"parse_cache\" src", kind: "search", status: "completed",
      content: [{ type: "content", content: { type: "text", text: "src/parser.rs:88: static PARSE_CACHE: OnceLock<Cache>\nsrc/parser.rs:141: PARSE_CACHE.get_or_init(Cache::new)" } }] } },
    { seq: 3, ts_ms: 3, update: { sessionUpdate: "tool_call", toolCallId: "t2", title: "Read src/parser.rs", kind: "read", status: "completed" } },
    { seq: 4, ts_ms: 4, update: { sessionUpdate: "tool_call", toolCallId: "t3", title: "cargo test parser::cache -- --test-threads 8", kind: "execute", status: "failed",
      content: [{ type: "content", content: { type: "text", text: "test parser::cache::reuses_entries ... FAILED\nthread panicked: entry evicted by a concurrent test" } }] } },
    { seq: 5, ts_ms: 5, update: { sessionUpdate: "agent_message_chunk", messageId: "m", content: { type: "text", text: "**Cause:** `PARSE_CACHE` is process-global, so tests running in parallel evict each other's entries.\n\nGive each test its own `Cache` through `Parser::with_cache`; no production change is needed." } } },
  ],
} as unknown as ObservedPage;

/** Leave the rows and bar drawn, with the inspector open at `open` (a task
 *  key, `"list"`, or nothing), for screenshots. */
export function renderBackgroundTasksPreview(desktop: boolean, open: string | null = null, observed = false): void {
  if (observed) setSubagentTranscriptTransport((_session, _child, cursor) => Promise.resolve(cursor.after !== undefined ? { events: [], more_before: false } : PREVIEW_TRANSCRIPT));
  if (open !== null) openBackgroundTask("s", open === "list" ? null : open, "row");
  const container = document.createElement("div");
  document.body.append(container);
  const background = backgroundOf(TIMELINE, [call(), call({
    call_id: "call-2",
    labels: { title: "Store review" },
    state: "completed",
    verdict: "needs-attention",
    finding_count: 2,
    has_result: true,
    finished_at_ms: Date.now() - 5_000,
  })]);
  if (desktop) {
    requestDesktopWorkView("tasks");
    createRoot(container).render(
      <SurfaceContext.Provider value={DESKTOP}>
        <BrowserProductTheme>
          <CssBaseline />
          <Box sx={{ width: "100vw", height: "100vh", display: "flex" }}>
            <DesktopWorkspaceProvider>
              <DesktopCommandProvider>
                <DesktopWorkspace
                  promptWidth={440}
                  resizing={false}
                  onResizeStart={() => undefined}
                  sessionId="s"
                  projection="history"
                  onProjectionChange={() => undefined}
                  prompt={
                    <Stack spacing={1.5} sx={{ p: 2, height: "100%" }}>
                      <Box sx={{ typography: "body2", color: "text.secondary" }}>
                        Draft to parent agent stays live beside the Tasks tab:
                      </Box>
                      <Box sx={{ flex: 1, p: 1.5, borderRadius: 1.5, border: 1, borderColor: "divider", typography: "body2" }}>
                        Does the retry case in Store review also affect restore paths?
                      </Box>
                    </Stack>
                  }
                  conversation={
                    <Stack spacing={1} sx={{ p: 2 }}>
                      {background.tasks.map((task) => <TaskRow key={task.key} task={task} sessionId="s" desktop />)}
                    </Stack>
                  }
                />
                <Box sx={{ position: "fixed", inset: 0, pointerEvents: "none", display: "none" }}>
                  <DesktopTasksView sessionId="s" active={false} providedBackground={background} />
                </Box>
              </DesktopCommandProvider>
            </DesktopWorkspaceProvider>
          </Box>
        </BrowserProductTheme>
      </SurfaceContext.Provider>,
    );
    return;
  }
  createRoot(container).render(
    <SurfaceContext.Provider value={MOBILE}>
      <BrowserProductTheme>
        <CssBaseline />
        <Stack spacing={1} sx={{ width: 390, p: 2 }}>
          {Array.from({ length: 6 }, (_, i) => (
            <Box key={i} sx={{ typography: "body2", color: "text.secondary" }}>
              Main conversation message {i + 1}: verifying retry idempotency and restore safety across workers.
            </Box>
          ))}
          {background.tasks.filter((task) => task.source !== "approval").map((task) => (
            <TaskRow key={task.key} task={task} sessionId="s" desktop={false} />
          ))}
        </Stack>
        <BackgroundTasksInspector sessionId="s" desktop={false} background={background} />
      </BrowserProductTheme>
    </SurfaceContext.Provider>,
  );
}
