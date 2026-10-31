/** Actual Calls dock and child notice, synthetic HTTP in an isolated browser. */
import { DocumentNotifications } from "./documents/DocumentNotifications";
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { COARSE_POINTER_ROOT_CLASS } from "./platform";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { ManagedCallsDock, ManagedChildNotice } from "./ManagedCallsDock";
import { CallTranscript } from "./CallTranscript";
import { getActiveSessionId, setActiveSessionId } from "./controlPlane";
import { composerStackExpandedStore } from "./composerStackAccordion";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import { type ManagedCallSummary, useManagedCalls } from "./managedCalls";
import { peekSession, retrySessionHydration } from "./store";
import { AgentToolsSettings, SessionToolsSection } from "./AgentToolsPanel";
import { CallApprovalOverlay } from "./CallApprovalOverlay";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import {
  type CallApproval,
  receiveCallApproval,
  useCallApproval,
} from "./callApproval";
import {
  type AgentTools,
  DEFAULT_REVIEW_PREFERENCES,
  overrideFor,
  type SessionToolsOverride,
} from "./agentTools";

const toolsDefaults: AgentTools = {
  schema: 1,
  memory: { tools: true, recall: true },
  calls: {
    enabled: false,
    targets: [{ agent: "codex" }],
    default: "auto",
    max_concurrent: 4,
    max_per_session: 64,
  },
};
/** A Codex kind's built-in defaults: it calls Claude, never itself. */
const codexDefaults: AgentTools = {
  ...toolsDefaults,
  calls: { ...toolsDefaults.calls, targets: [{ agent: "claude-code" }] },
};
const toolsCatalog = {
  call_targets: [
    {
      agent: "codex",
      presets: [{
        id: "astra-max",
        name: "Astra · Max",
        detail: "",
        is_default: false,
        values: { model: "gpt-6-astra", reasoning_effort: "max" },
      }],
      options: [
        {
          id: "model",
          name: "Model",
          choices: [
            { value: "gpt-6-astra", name: "GPT-6-Astra" },
            { value: "gpt-6.1-sol", name: "GPT-6.1-Sol" },
          ],
        },
        {
          id: "reasoning_effort",
          name: "Reasoning",
          choices: [
            { value: "high", name: "High" },
            { value: "max", name: "Max" },
          ],
        },
      ],
    },
    {
      agent: "claude-code",
      presets: [{
        id: "opus-high",
        name: "Opus · High",
        detail: "",
        is_default: false,
      }],
    },
  ],
};

function overlay(
  defaults: AgentTools,
  override: SessionToolsOverride,
): AgentTools {
  return {
    ...defaults,
    memory: { ...defaults.memory, ...override.memory },
    calls: { ...defaults.calls, ...override.calls, review: {
      ...(defaults.calls.review ?? DEFAULT_REVIEW_PREFERENCES), ...override.calls?.review,
    } },
  };
}

/** A child conversation the store hydrates through its bootstrap route. */
function childBootstrap(url: string): Response | null {
  const match = /^\/api\/sessions\/([^/]+)\/bootstrap$/.exec(url);
  if (!match) return null;
  const session = decodeURIComponent(match[1]!);
  const update = (seq: number, sessionUpdate: string, text: string) => ({
    session_id: session,
    seq,
    kind: "update",
    update: { sessionUpdate, content: { type: "text", text } },
  });
  return Response.json({
    messages: [{
      type: "snapshot",
      session_id: session,
      reached_start: true,
      events: [
        update(1, "user_message_chunk", "Review the billing retry change."),
        {
          session_id: session,
          seq: 2,
          kind: "update",
          update: {
            sessionUpdate: "tool_call",
            toolCallId: "child-tool-1",
            title: "rg -n retry internal/billing",
            kind: "execute",
            status: "completed",
            rawInput: { command: "rg -n retry internal/billing" },
          },
        },
        update(
          3,
          "agent_message_chunk",
          "The retry path in `charge.go` resubmits after the provider accepted " +
            "the request, so a charge can be **duplicated**.\n\n" +
            "The outbound client also has no deadline.\n\n" +
            // One unbreakable line: it must scroll inside its block, never
            // widen the modal's conversation column.
            "```text\n" + "node-heredoc-".repeat(120) + "\n```\n\n" +
            Array.from(
              { length: 40 },
              (_, line) => `Checked step ${line + 1} of the retry path.`,
            ).join("\n\n") + "\n\nReview complete.",
        ),
      ],
    }],
  });
}

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function until<T>(
  read: () => T | null | undefined,
  label: string,
): Promise<T> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const value = read();
    if (value) return value;
    await wait(30);
  }
  throw new Error(label);
}

function summary(
  call: string,
  state: ManagedCallSummary["state"],
  aspect: string,
  extra: Partial<ManagedCallSummary> = {},
): ManagedCallSummary {
  return {
    call_id: call,
    request_id: `review-${call}`,
    provider: "codex",
    provider_account_id: call === "call-security" ? null : "default",
    purpose: "review",
    labels: { group: "PR 7961 review", round: "1", aspect },
    placement: {
      parent_session_id: "parent",
      machine_id: "hawk",
      workspace_id: "marketplace-service",
    },
    child_session_id: `child-${call}`,
    state,
    created_at_ms: Date.now() - 95_000,
    updated_at_ms: Date.now() - 1_000,
    input_revision: `sha256:${"ab".repeat(32)}`,
    has_result: state === "completed",
    runtime_machine_id: "ovh",
    provider_version: "3.4.0",
    cancel_requested: false,
    error: state === "failed" ? { code: "structured_output_invalid" } : null,
    ...extra,
  };
}

const findingsResult = {
  text: "{}",
  truncated: false,
  stop_reason: "end_turn",
  usage: {
    input_tokens: 1000,
    cached_input_tokens: 900,
    uncached_input_tokens: 100,
    output_tokens: 50,
    total_tokens: 1050,
    reasoning_tokens: 10,
  },
  structured: {
    verdict: "needs-attention",
    summary: "Retries can duplicate a charge.",
    findings: [{
      severity: "high",
      title: "Retry duplicates the charge",
      body: "The handler retries after the provider accepted the request.",
      file: "internal/billing/charge.go",
      line_start: 42,
      line_end: 48,
      confidence: 0.8,
      recommendation: "Persist an idempotency key.",
    }, {
      severity: "medium",
      title: "Missing timeout",
      body: "The outbound call has no deadline.",
      file: "internal/billing/client.go",
      line_start: 7,
      line_end: 9,
      confidence: 0.6,
      recommendation: "Bound the request.",
    }],
    next_steps: ["Add a regression test."],
  },
};

export async function runManagedCallsBrowserConformance(): Promise<string[]> {
  bindProductSyncPrincipal("call-account-fixture");
  const originalFetch = globalThis.fetch;
  const tests: string[] = [];
  let recoveryText = "Checking current keyboard and queue behavior.";
  let recoveryReads = 0;
  let delayedRecovery: (() => void) | undefined;
  let delayRecovery = false;
  let clearSnapshot = false;
  const clearAt = Date.now();
  let calls: ManagedCallSummary[] = [];
  const dismissed = new Set<string>();
  const cancels: string[] = [];
  const cancelCount = (): number => cancels.length;
  let sessionOverride: SessionToolsOverride = { schema: 1 };
  const sessionWrites: SessionToolsOverride[] = [];
  let agentDefaults: AgentTools = codexDefaults;
  const agentWrites: (AgentTools | null)[] = [];
  const decisions: string[] = [];
  const sessionTools = () => ({
    session_id: "parent",
    agent: "claude-code",
    defaults: toolsDefaults,
    defaults_customized: false,
    override: sessionOverride,
    effective: overlay(toolsDefaults, sessionOverride),
    catalog: toolsCatalog,
  });
  const provider = managementEntryFixture("codex");
  provider.manifest.display.name = "Codex";
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    // A focused bootstrap names the client's cached high-water seq
    // (`?since_seq=`, docs/background-tasks.md); route it by its path.
    const url = String(input).replace(/(\/bootstrap)\?since_seq=\d+$/, "$1");
    if (url === "/api/provider-accounts") return Promise.resolve(Response.json({
      dataset: { dataset_id: "calls-fixture", user_id: "call-account-fixture" },
      registry: { revision: 1, accounts: [], defaults: {} }, session_admission: "enabled",
      display_preferences: { scope: "subscriptions", revision: 1, values: { legacy_labels: { codex: "OpenAI · Pika" } } },
    }));
    if (url === "/api/sessions/parent-clear/bootstrap") {
      return Promise.resolve(Response.json({ messages: [{
        type: "snapshot", session_id: "parent-clear", reached_start: true,
        events: clearSnapshot ? [{ session_id: "parent-clear", seq: 10, kind: "update",
          update: { sessionUpdate: "context_cleared", at: clearAt } }] : [],
      }] }));
    }
    if (url === "/api/sessions/parent-clear/calls") {
      // Deliberately retain pre-clear rows, as a poll racing durable archival can.
      return Promise.resolve(Response.json({ calls: [
        summary("old-running", "running", "old", { created_at_ms: clearAt - 2 }),
        summary("old-ended", "completed", "old", { created_at_ms: clearAt - 1 }),
        ...(clearSnapshot ? [summary("fresh", "running", "fresh", { created_at_ms: clearAt + 1 })] : []),
      ] }));
    }
    if (url === "/api/sessions/child-recovery/bootstrap") {
      recoveryReads += 1;
      const response = Response.json({ messages: [{
        type: "snapshot",
        session_id: "child-recovery",
        reached_start: true,
        events: [{
          session_id: "child-recovery",
          seq: 78,
          kind: "update",
          update: {
            sessionUpdate: "agent_message_chunk",
            messageId: "review-answer",
            content: { type: "text", text: recoveryText },
          },
        }],
      }] });
      if (delayRecovery) {
        delayRecovery = false;
        return new Promise<Response>((resolve) => {
          delayedRecovery = () => resolve(response);
        });
      }
      return Promise.resolve(response);
    }
    const bootstrap = childBootstrap(url);
    if (bootstrap) return Promise.resolve(bootstrap);
    if (url === "/api/plugins") {
      return Promise.resolve(
        Response.json({
          providers: [provider],
          authentications: [],
          authentication_executors: [],
        }),
      );
    }
    if (url === "/api/sessions/parent/calls/approval") {
      decisions.push(
        (JSON.parse(String(init?.body)) as { decision: string }).decision,
      );
      return Promise.resolve(new Response(null, { status: 204 }));
    }
    if (url === "/api/sessions/parent/tools") {
      if (init?.method === "PUT") {
        sessionOverride = JSON.parse(String(init.body)) as SessionToolsOverride;
        sessionWrites.push(sessionOverride);
      }
      return Promise.resolve(Response.json(sessionTools()));
    }
    if (url === "/api/agent-tools") {
      return Promise.resolve(Response.json({
        schema: 1,
        agents: [{
          agent: "codex",
          settings: agentDefaults,
          customized: agentDefaults !== codexDefaults,
        }],
        catalog: toolsCatalog,
      }));
    }
    if (url === "/api/agent-tools/codex" && init?.method === "PUT") {
      const body = JSON.parse(String(init.body)) as {
        settings: AgentTools | null;
      };
      agentWrites.push(body.settings);
      agentDefaults = body.settings ?? codexDefaults;
      return Promise.resolve(Response.json({
        schema: 1,
        agent: "codex",
        settings: agentDefaults,
        customized: body.settings !== null,
      }));
    }
    if (url === "/api/sessions/parent/calls") {
      return Promise.resolve(
        Response.json({ schema: 1, calls: calls.filter((call) => !dismissed.has(call.call_id)), next_before: null }),
      );
    }
    if (url === "/api/sessions/parent/calls/dismiss" && init?.method === "POST") {
      const body = JSON.parse(String(init.body)) as { call_ids: string[]; dismissed: boolean };
      check(!body.call_ids.includes("call-security"), "active call must not be selected for cleanup");
      for (const id of body.call_ids) {
        if (body.dismissed) dismissed.add(id);
        else dismissed.delete(id);
      }
      return Promise.resolve(Response.json(body));
    }
    const detail = /^\/api\/sessions\/parent\/calls\/([^/]+)$/.exec(url);
    if (detail) {
      const found = calls.find((call) => call.call_id === detail[1]);
      return Promise.resolve(
        found
          ? Response.json({
            schema: 1,
            call: {
              ...found,
              result: found.state === "completed" ? findingsResult : null,
            },
          })
          : new Response("not found", { status: 404 }),
      );
    }
    const cancel = /^\/api\/sessions\/parent\/calls\/([^/]+)\/cancel$/.exec(
      url,
    );
    if (cancel && init?.method === "POST") {
      cancels.push(cancel[1]!);
      calls = calls.map((call) =>
        call.call_id === cancel[1] ? { ...call, cancel_requested: true } : call
      );
      return Promise.resolve(Response.json({ schema: 1 }));
    }
    return Promise.resolve(new Response("not found", { status: 404 }));
  }) as typeof fetch;
  resetProviderCatalog();
  const container = document.createElement("div");
  container.style.width = "560px";
  document.body.append(container);
  let root: Root = createRoot(container);
  // The product's notice host: Calls cleanup offers its Undo there.
  const render = (node: React.ReactNode) =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <SurfaceProvider>
            {node}
            <DocumentNotifications />
          </SurfaceProvider>
        </BrowserProductTheme>,
      )
    );
  const text = () => document.body.textContent ?? "";
  const button = (label: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find((node) =>
      node.getAttribute("aria-label") === label ||
      node.textContent?.trim() === label
    );
  try {
    function ClearCallsProbe(): React.JSX.Element {
      const observed = useManagedCalls("parent-clear");
      return <div data-clear-calls>{observed.calls.map((call) => call.call_id).join(",")}</div>;
    }
    const releaseClear = peekSession("parent-clear");
    try {
      render(<ClearCallsProbe />);
      await until(() => container.textContent === "old-running,old-ended", "old calls fixture");
      clearSnapshot = true;
      await retrySessionHydration("parent-clear");
      await until(() => container.textContent === "fresh", "Clear must hide old running and ended calls while showing new calls");
      render(<></>);
    } finally {
      releaseClear();
    }
    tests.push("clear-hides-old-calls-even-when-a-poll-races-archival");
    render(<CallTranscript sessionId="child-recovery" provider="claude-code" />);
    await until(() => text().includes(recoveryText), "initial review progress");
    render(<></>);
    // Reopening must refresh even an already-hydrated conversation. Keep this
    // old response in flight while completion arrives through call metadata.
    delayRecovery = true;
    render(<CallTranscript sessionId="child-recovery" provider="claude-code" />);
    await until(() => delayedRecovery, "cached review was not refreshed on reopen");
    recoveryText = "Final review: not ready to implement; nine blocking issues.";
    render(<CallTranscript sessionId="child-recovery" provider="claude-code" settled />);
    await until(() => text().includes(recoveryText), "completion did not recover final review");
    delayedRecovery!();
    await wait(100);
    check(text().includes(recoveryText), "stale in-flight tail replaced the final review");
    check(recoveryReads === 3, "recovery should fetch on open, reopen and completion only");
    render(<CallTranscript sessionId="child-recovery" provider="claude-code" settled desktop />);
    await wait(100);
    check(recoveryReads === 3, "ordinary rerenders must not refetch the transcript");
    render(<></>);
    recoveryText = "Reopened review includes the complete final conclusion.";
    render(<CallTranscript sessionId="child-recovery" provider="claude-code" settled />);
    await until(() => text().includes(recoveryText), "settled review remained stale on reopen");
    render(<></>);
    tests.push("review-recovers-final-output-on-completion-and-reopen-without-stale-overwrite");
    composerStackExpandedStore().set(null);
    render(<ManagedCallsDock sessionId="parent" desktop />);
    await wait(150);
    check(container.textContent === "", "dock without calls must not render");
    tests.push("hidden-without-calls");

    root.unmount();
    root = createRoot(container);
    calls = [summary("call-reused", "completed", "correctness", {
      child_session_id: "unused-reuse-child",
      reused_from: "call-original",
      reused_child_session_id: "child-original",
    })];
    composerStackExpandedStore().set("calls");
    render(<ManagedCallsDock sessionId="parent" desktop />);
    (await until(
      () => document.querySelector<HTMLElement>("[data-desktop-item='call-call-reused']"),
      "reused call missing",
    )).click();
    const reusedModal = await until(
      () => document.querySelector<HTMLElement>("[data-desktop-call-modal='call-reused']"),
      "reused call modal missing",
    );
    await until(
      () => reusedModal.querySelector("[data-call-transcript='child-original']"),
      "reused call must show the original transcript",
    );
    check(!reusedModal.querySelector("[data-call-transcript='unused-reuse-child']"),
      "reuse must not show an unused child conversation");
    reusedModal.dispatchEvent(new KeyboardEvent("keydown", { key: "o", bubbles: true }));
    check(getActiveSessionId() === "child-original",
      "the open-session shortcut must follow the reused conversation");
    tests.push("reused-call-opens-original-transcript-and-session");
    setActiveSessionId("parent");
    composerStackExpandedStore().set(null);
    root.unmount();
    root = createRoot(container);
    calls = [
      summary("call-security", "running", "security", { created_at_ms: Date.now() - 300_000 }),
      summary("call-correctness", "completed", "correctness", {
        configuration: { model: "gpt-6-astra", reasoning_effort: "max" },
        preset: "astra-max",
        created_at_ms: Date.now() - 100_000,
        started_at_ms: Date.now() - 90_000,
        finished_at_ms: Date.now() - 1_000,
        labels: { group: "PR 7961 review", round: "1", aspect: "correctness", selection_reason: "Ample quota; strongest available reviewer" },
        verdict: "needs-attention",
        finding_count: 2,
      }),
      summary("call-tests", "failed", "test-coverage", {
        created_at_ms: Date.now() - 200_000,
        input_revision: null,
        error: { code: "unsupported_input", detail: "symlink_unsupported" },
      }),
    ];
    render(<ManagedCallsDock sessionId="parent" desktop />);
    await until(
      () => button("Expand calls"),
      "desktop calls header did not appear",
    );
    check(
      text().toLowerCase().includes("security · round 1"),
      `active aspect missing from summary: ${text().slice(0, 400)}`,
    );
    check(text().includes("2 findings"), "findings count missing from summary");
    check(text().includes("2/3"), "settled/total count missing");
    tests.push("desktop-summary-separates-execution-from-findings");

    const chip = await until(
      () => document.querySelector<HTMLElement>("[data-call-chip='call-security']"),
      "a running call must show as a chip while Calls is collapsed",
    );
    check(
      !document.querySelector("[data-call-chip='call-correctness']"),
      "only running calls get a chip",
    );
    chip.click();
    await until(
      () => document.querySelector("[data-desktop-call-modal='call-security']"),
      "the running chip must open its call directly",
    );
    check(button("Expand calls"), "opening a chip must not expand the list");
    button("Close call")!.click();
    await until(
      () => document.querySelector("[data-desktop-call-modal]") ? null : true,
      "the call modal did not close",
    );
    tests.push("desktop-running-call-chip-opens-call");

    button("Expand calls")!.click();
    await until(() => button("Collapse calls"), "calls did not expand");
    const rows = document.querySelectorAll("[data-desktop-item^='call-']");
    check(rows.length === 3, "every call must be a keyboard item");
    check(
      text().includes("OVH → Hawk"),
      "runtime and execution placement missing",
    );
    check(text().includes("PR 7961 review"), "caller group label missing");
    check(
      Array.from(rows).every((row) =>
        /\d{2}:\d{2} · \d+(m \d+)?s/.test(row.textContent ?? "") &&
        row.querySelector("[title^='Created ']")
      ),
      "every row must show when its call started and how long it ran",
    );
    tests.push("desktop-list-shows-placement-and-group");

    (rows[1] as HTMLElement).click();
    const modal = await until(
      () =>
        document.querySelector<HTMLElement>(
          "[data-desktop-call-modal='call-correctness']",
        ),
      "a call must open in a modal",
    );
    check(
      modal.querySelector("[data-call-transcript='child-call-correctness']"),
      "the modal must show the child's own transcript",
    );
    await until(
      () => text().includes("Retry duplicates the charge") ? true : null,
      "full result not loaded",
    );
    check(
      modal.textContent?.includes("New input") &&
        modal.textContent.includes("Cached input") &&
        modal.textContent.includes("1,050") &&
        modal.textContent.includes("included in output"),
      "call details must distinguish fresh, cached and total tokens without adding reasoning twice",
    );
    await until(
      () => modal.textContent?.includes("node-heredoc-") ? true : null,
      "the child transcript did not render",
    );
    const bounds = modal.getBoundingClientRect();
    for (
      const part of [
        "[data-call-conversation='call-correctness']",
        "[data-call-details-panel]",
      ]
    ) {
      const edge = modal.querySelector(part)?.getBoundingClientRect().right;
      check(
        edge !== undefined && edge <= bounds.right + 0.5,
        `${part} overflows the modal (${edge} > ${bounds.right})`,
      );
    }
    modal.dispatchEvent(
      new KeyboardEvent("keydown", { key: "i", bubbles: true }),
    );
    await until(
      () => modal.querySelector("[data-call-details-panel]") ? null : true,
      "i must hide the details panel",
    );
    modal.dispatchEvent(
      new KeyboardEvent("keydown", { key: "i", bubbles: true }),
    );
    await until(
      () => modal.querySelector("[data-call-details-panel]"),
      "i must show the details panel again",
    );
    await until(
      () => text().includes("internal/billing/charge.go:42") ? true : null,
      "finding location missing",
    );
    check(
      text().includes("recorded by the calling workflow"),
      "disposition ownership note missing",
    );
    check(text().includes("Input snapshot"), "snapshot identity missing");
    tests.push("desktop-modal-shows-transcript-and-details");

    const paper = modal.closest<HTMLElement>(".MuiDialog-paper")!
      .getBoundingClientRect();
    check(
      paper.width >= innerWidth - 40 && paper.height >= innerHeight - 40,
      `the call modal must fill the viewport (${paper.width}x${paper.height})`,
    );
    check(
      modal.querySelectorAll("[data-call-rail-item]").length === 3,
      "the call modal must list every call",
    );
    modal.dispatchEvent(new KeyboardEvent("keydown", { key: "]", bubbles: true }));
    await until(
      () => document.querySelector("[data-desktop-call-modal='call-tests']"),
      "] must step to the next call",
    );
    document.querySelector<HTMLElement>("[data-desktop-call-modal]")!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "[", bubbles: true }));
    const stepped = await until(
      () => document.querySelector<HTMLElement>("[data-desktop-call-modal='call-correctness']"),
      "[ must step to the previous call",
    );
    tests.push("desktop-modal-fills-viewport-and-steps-calls");

    (await until(
      () => stepped.querySelector<HTMLElement>("[data-call-transcript] [aria-haspopup='dialog']"),
      "child tool row missing",
    )).click();
    await until(
      () => text().includes("Tool history inspector") ? true : null,
      "the tool inspector did not open",
    );
    check(
      document.elementFromPoint(innerWidth / 2, innerHeight / 2)
        ?.closest("[role='dialog']")?.textContent?.includes("Tool history inspector"),
      "the tool inspector must open above the call modal",
    );
    // React bubbles the inspector's portal events through the call modal.
    [...document.querySelectorAll<HTMLElement>("[role='dialog']")]
      .find((node) => node.textContent?.includes("Tool history inspector"))!
      .dispatchEvent(new KeyboardEvent("keydown", { key: "i", bubbles: true }));
    check(
      stepped.querySelector("[data-call-details-panel]"),
      "inspector keys must not reach the call modal",
    );
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    await until(
      () => text().includes("Tool history inspector") ? null : true,
      "Escape must close the tool inspector",
    );
    check(
      document.querySelector("[data-desktop-call-modal='call-correctness']"),
      "closing the inspector must keep the call modal",
    );
    tests.push("desktop-tool-inspector-opens-above-call-modal");

    (rows[2] as HTMLElement).click();
    const failed = await until(
      () => document.querySelector("[data-call-conversation='call-tests']"),
      "failed call did not open",
    );
    check(
      failed.querySelector("[role='alert']")?.textContent?.includes(
        "unsupported_input: symlink_unsupported",
      ),
      "launch refusal must show its cause in the conversation pane",
    );
    check(
      !failed.querySelector("[data-call-transcript]") &&
        failed.textContent?.includes("No conversation was started"),
      "a pre-snapshot failure must not try to restore a nonexistent child",
    );
    tests.push("pre-snapshot-failure-shows-cause-without-transcript");

    (rows[0] as HTMLElement).click();
    await until(
      () => button("Stop call"),
      "stop action missing for a running call",
    );
    button("Stop call")!.click();
    const confirm = await until(
      () => {
        // The call modal is a dialog too; the confirmation opens after it.
        const stops = [
          ...document.querySelectorAll<HTMLButtonElement>(
            "[role='dialog'] button",
          ),
        ].filter((node) => node.textContent?.trim() === "Stop call");
        return stops.length > 1 ? stops.at(-1) : null;
      },
      "stop confirmation missing",
    );
    check(cancelCount() === 0, "stop must wait for confirmation");
    confirm.click();
    await until(
      () => cancels.length === 1 ? true : null,
      "stop request not sent",
    );
    await wait(100);
    check(
      cancels.length === 1 && cancels[0] === "call-security",
      "stop must be sent exactly once",
    );
    tests.push("stop-requires-confirmation-and-posts-once");

    setActiveSessionId("parent");
    button("Open as session")!.click();
    check(
      getActiveSessionId() === "child-call-security",
      "open as session must select the child",
    );
    await until(
      () => document.querySelector("[data-desktop-call-modal]") ? null : true,
      "opening the child must close the modal",
    );
    tests.push("open-conversation-selects-child");

    root.unmount();
    root = createRoot(container);
    render(<ManagedCallsDock sessionId="parent" desktop={false} />);
    const open = await until(
      () => button("Open calls"),
      "mobile calls header missing",
    );
    check(
      open.getBoundingClientRect().height >= 44,
      "mobile header must be a large touch target",
    );
    const previewRail = container.querySelector<HTMLElement>("[data-call-preview-rail]")!;
    check(previewRail && getComputedStyle(previewRail).overflowX === "auto", "mobile previews provide a horizontal scroller");
    check(!previewRail.closest("button"), "preview buttons must not nest inside the overview button");
    const preview = previewRail.querySelector<HTMLButtonElement>("[data-call-preview]")!;
    check([...previewRail.querySelectorAll("[data-call-preview]")].map((card) => card.getAttribute("data-call-preview")).join(",") === "call-correctness,call-tests,call-security",
      "mobile previews sort newest first regardless of running, failed or completed state");
    check(preview.querySelector("time[datetime]") !== null,
      "call previews expose when execution was requested or started");
    const age = preview.querySelector<HTMLElement>("time[datetime]")!;
    const titleRow = preview.firstElementChild!.firstElementChild!;
    check(!titleRow.contains(age) && age.parentElement === preview.firstElementChild!.lastElementChild,
      "relative execution time belongs on the final preview row");
    check(/^· (now|(?:\d+d )?(?:\d+h )?(?:\d+m )?ago)$/.test(age.textContent ?? ""),
      "relative execution time distinguishes age from duration");
    check(!preview.textContent?.includes("Hawk") && preview.title.includes("Created") && preview.title.includes("Hawk"),
      "call previews reserve visible space for execution settings and keep placement in details");
    const configuredPreview = previewRail.querySelector<HTMLElement>('[data-call-preview="call-correctness"]')!;
    check(configuredPreview.querySelector("[data-call-preview-configuration]")?.textContent?.includes("gpt-6-astra") &&
      configuredPreview.querySelector("[data-call-preview-reasoning]")?.textContent?.includes("max"),
      "call previews show recorded model and reasoning on a dedicated configuration row");
    check(configuredPreview.querySelector("[data-call-preview-configuration]") === configuredPreview.firstElementChild!.children[1],
      "configuration must occupy the middle row between title and status");
    check(configuredPreview.querySelector("[data-account-badge]")?.textContent === "Pika" &&
      configuredPreview.firstElementChild!.children.length === 3,
      "recorded execution account appears without adding a fourth preview row");
    check(previewRail.querySelector('[data-call-preview="call-security"] [data-account-badge="unknown"]') !== null,
      "historical calls without account evidence never inherit today's default");
    check(previewRail.querySelector('[data-call-preview="call-security"] [data-call-preview-configuration]')?.textContent?.includes("Model not recorded"),
      "legacy calls explicitly explain missing model history");
    check(configuredPreview.textContent?.includes("Done") && !configuredPreview.textContent.includes("Completed"),
      "completed calls use the compact Done label");
    configuredPreview.style.width = "14rem";
    const reasoningLabel = configuredPreview.querySelector<HTMLElement>("[data-call-preview-reasoning]")!;
    check(reasoningLabel.scrollWidth <= reasoningLabel.clientWidth &&
      reasoningLabel.getBoundingClientRect().right <= configuredPreview.getBoundingClientRect().right &&
      configuredPreview.firstElementChild?.children.length === 3,
      "minimum-width preview retains readable reasoning within three rows");
    configuredPreview.style.width = "";
    // Touch dismissal can return focus to the opener with Mui-focusVisible.
    // Its permanent card fill must survive the theme's latched-hover reset.
    const previewSurface = preview.firstElementChild as HTMLElement;
    const previewFill = () => {
      const inner = getComputedStyle(previewSurface).backgroundColor;
      return inner === "rgba(0, 0, 0, 0)" ? getComputedStyle(preview).backgroundColor : inner;
    };
    const restingPreviewFill = previewFill();
    const hadCoarse = document.documentElement.classList.contains(COARSE_POINTER_ROOT_CLASS);
    document.documentElement.classList.add(COARSE_POINTER_ROOT_CLASS);
    preview.classList.add("Mui-focusVisible");
    check(restingPreviewFill !== "rgba(0, 0, 0, 0)" && previewFill() === restingPreviewFill,
      "touch focus must preserve the Calls preview card background");
    preview.classList.remove("Mui-focusVisible");
    if (!hadCoarse) document.documentElement.classList.remove(COARSE_POINTER_ROOT_CLASS);
    tests.push("mobile-preview-keeps-fill-after-touch-focus");
    const cards = [...previewRail.querySelectorAll<HTMLElement>("[data-call-preview]")];
    const previousWidth = container.style.width;
    container.style.width = "900px";
    cards.slice(1).forEach((card) => card.style.display = "none");
    const cardStyle = getComputedStyle(preview);
    check(preview.getBoundingClientRect().width < previewRail.clientWidth &&
      preview.getBoundingClientRect().width >= parseFloat(cardStyle.minWidth) &&
      preview.getBoundingClientRect().width <= parseFloat(cardStyle.maxWidth),
      "one preview fits its content within minimum and maximum widths instead of filling the rail");
    check(age.getBoundingClientRect().left - age.previousElementSibling!.getBoundingClientRect().right <= 8,
      "relative time follows the metadata without an expanding gap");
    const title = titleRow.querySelector("[data-call-preview-title]")!;
    const originalTitle = title.textContent;
    title.textContent = "A much longer call title ".repeat(12);
    check(Math.abs(preview.getBoundingClientRect().width - parseFloat(cardStyle.maxWidth)) <= 1,
      "long call titles stop growing at the maximum card width");
    title.textContent = originalTitle;
    cards.slice(1).forEach((card) => card.style.display = "");
    container.style.width = "390px";
    cards.slice(2).forEach((card) => card.style.display = "none");
    check(previewRail.scrollWidth > previewRail.clientWidth &&
      cards[1]!.getBoundingClientRect().right > previewRail.getBoundingClientRect().right,
      "phone previews retain their width and overflow into horizontal scrolling");
    cards.slice(2).forEach((card) => card.style.display = "");
    container.style.width = previousWidth;
    for (const [type, x] of [["pointerdown", 100], ["pointermove", 40], ["pointerup", 40]] as const) {
      preview.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 77, pointerType: "touch", isPrimary: true, clientX: x, clientY: 10 }));
    }
    check(!document.querySelector("[data-call-transcript]"), "horizontal drag must not open a call");
    preview.click();
    await until(() => document.querySelector("[data-call-transcript]"), "preview opens the call directly");
    (await until(() => button("Close"), "preview close button missing"))!.click();
    await until(() => document.querySelector("[data-call-transcript]") ? null : true, "preview closes cleanly");
    open.click();
    const row = await until(
      () =>
        [...document.querySelectorAll<HTMLButtonElement>("button")].find((
          node,
        ) => !node.hasAttribute("data-call-preview") && node.getAttribute("aria-label")?.includes("correctness")),
      "mobile list missing",
    );
    check(
      row.getBoundingClientRect().height >= 56,
      "mobile rows must be large touch targets",
    );
    row.click();
    // Level 2: the child's transcript, with Back beside Close.
    await until(
      () =>
        document.querySelector(
          "[data-call-transcript='child-call-correctness']",
        ),
      "a mobile call must open its transcript",
    );
    const executionHeader = document.querySelector<HTMLElement>("[data-call-header-configuration]");
    const callHeader = document.querySelector<HTMLElement>("[data-call-header]")!;
    check(executionHeader?.textContent?.includes("gpt-6-astra · max") &&
      !callHeader.textContent?.includes("Not recorded") &&
      !executionHeader.textContent.includes("astra-max"),
      "mobile transcript header shows recorded parameters without missing-value or redundant preset labels");
    check(callHeader.children.length === 2 && executionHeader!.parentElement === callHeader.querySelector("[data-call-header-metadata]"),
      "mobile transcript header has a title row and a separate row containing execution parameters and status metadata");
    const headerAge = callHeader.querySelector<HTMLElement>("[data-call-header-age]")!;
    check(headerAge.textContent?.endsWith("ago") || headerAge.textContent?.endsWith("now"),
      "mobile transcript header uses the same relative time format as previews");
    callHeader.style.width = "320px";
    check(callHeader.scrollWidth <= callHeader.clientWidth,
      "two-line call header must fit a narrow phone without horizontal overflow");
    const metadataRow = callHeader.querySelector<HTMLElement>("[data-call-header-metadata]")!;
    for (const selector of ["[data-call-header-configuration]", "[data-call-header-placement]"]) {
      const badge = callHeader.querySelector<HTMLElement>(`${selector} > span`);
      check(badge && getComputedStyle(badge).backgroundColor !== "rgba(0, 0, 0, 0)" &&
        parseFloat(getComputedStyle(badge).borderRadius) > 0 && !badge.closest("button"),
        "recorded parameters and placement use filled, noninteractive badges");
    }
    check(getComputedStyle(metadataRow).overflowX === "auto" &&
      getComputedStyle(executionHeader!).whiteSpace === "nowrap",
      "long execution metadata scrolls within the second row without wrapping or widening the header");
    callHeader.style.width = "";
    const backButton = (): HTMLElement | null =>
      document.querySelector<HTMLElement>(
        "[data-mobile-sheet-footer-shield] button[aria-label='Back']",
      );
    check(backButton(), "a nested level must offer Back in the island");
    const island = (label: string): HTMLElement | null =>
      document.querySelector<HTMLElement>(
        `[data-mobile-sheet-footer-shield] button[aria-label='${label}']`,
      );
    check(
      island("Scroll to start") && island("Details") &&
        island("Scroll to latest"),
      "the transcript island must offer start, details and latest",
    );
    const visible = (needle: string): boolean => {
      const box = document.querySelector("[data-call-transcript]")!
        .getBoundingClientRect();
      return [...document.querySelectorAll("[data-call-transcript] *")].some(
        (node) => {
          if (node.children.length > 0 || !node.textContent?.includes(needle)) {
            return false;
          }
          const rect = node.getBoundingClientRect();
          return rect.bottom > box.top && rect.top < box.bottom;
        },
      );
    };
    await until(
      () => visible("Review complete.") ? true : null,
      "a call transcript must open at its latest message",
    );
    island("Scroll to start")!.click();
    await until(
      () => visible("Review the billing retry change.") ? true : null,
      "scroll to start must show the first message",
    );
    island("Scroll to latest")!.click();
    await until(
      () => visible("Review complete.") ? true : null,
      "scroll to latest must return to the newest message",
    );
    await until(
      () => text().includes("can be duplicated") ? true : null,
      `the child's own messages must render: ${text().slice(0, 300)}`,
    );
    check(
      !text().includes("Retry duplicates the charge"),
      "details stay one level deeper than the transcript",
    );
    // Level 3: details.
    (await until(() => button("Details"), "details entry missing")).click();
    await until(
      () => text().includes("Retry duplicates the charge") ? true : null,
      "mobile detail result missing",
    );
    check(text().includes("gpt-6-astra · max") && text().includes("Finished") && text().includes("Ample quota"), "details show model, effort, finish and choice reason");
    backButton()!.click();
    await until(
      () =>
        document.querySelector(
            "[data-call-transcript='child-call-correctness']",
          ) && !text().includes("Retry duplicates the charge")
          ? true
          : null,
      "back must return from details to the transcript",
    );
    backButton()!.click();
    await until(
      () => backButton() ? null : true,
      "back did not return to the list",
    );
    const failedRow = await until(
      () =>
        [...document.querySelectorAll<HTMLButtonElement>("button")].find((
          node,
        ) => node.getAttribute("aria-label")?.includes("test coverage")),
      "mobile failed call missing",
    );
    failedRow.click();
    await until(
      () => document.querySelector("[data-call-conversation='call-tests']"),
      "mobile failed call did not open",
    );
    check(
      text().includes("unsupported_input: symlink_unsupported") &&
        !document.querySelector("[data-call-transcript]"),
      "mobile must show the launch cause without a restoration skeleton",
    );
    backButton()!.click();
    await until(
      () => backButton() ? null : true,
      "failed call back did not return to list",
    );

    button("Clear ended calls…")!.click();
    await until(() => button("Clear 2"), "cleanup includes both terminal calls");
    check(dismissed.size === 0, "cleanup waits for confirmation");
    button("Clear 2")!.click();
    await until(() => button("Undo"), "cleanup must offer undo");
    check(dismissed.has("call-tests") && dismissed.has("call-correctness"), "selected ended calls cleared");
    button("Undo")!.click();
    await until(() => dismissed.size === 0 ? true : null, "undo restores cleared calls");
    tests.push("terminal-call-cleanup-is-confirmed-and-reversible");
    // Like every Cowboy cover sheet, the page closes from the frosted
    // floating island, not from a header control.
    const dismiss = await until(
      () =>
        document.querySelector<HTMLElement>(
          "[data-mobile-sheet-footer-shield] button[aria-label='Close']",
        ),
      "mobile calls page lacks the floating close island",
    );
    check(
      document.querySelectorAll("button[aria-label='Close']").length === 1,
      "mobile calls page must have exactly one close control",
    );
    dismiss.click();
    await until(
      () => button("correctness") ? null : true,
      "floating close did not dismiss the calls page",
    );
    // Closing from inside a call leaves the whole page: the next open starts
    // at the list, not at the call that was showing.
    const callRow = (): HTMLButtonElement | undefined =>
      [...document.querySelectorAll<HTMLButtonElement>("button")].find((
        node,
      ) => !node.hasAttribute("data-call-preview") && node.getAttribute("aria-label")?.includes("correctness"));
    button("Open calls")!.click();
    (await until(callRow, "reopened calls page lacks its list")).click();
    await until(backButton, "a reopened call must show its transcript");
    island("Close")!.click();
    await until(
      () => island("Close") ? null : true,
      "close inside a call did not dismiss the calls page",
    );
    button("Open calls")!.click();
    await until(callRow, "reopening after a close inside a call lacks the list");
    check(!backButton(), "reopening must start at the list, not the call");
    island("Close")!.click();
    await until(
      () => island("Close") ? null : true,
      "floating close did not dismiss the reopened calls page",
    );
    tests.push("mobile-page-list-transcript-details-and-back");

    root.unmount();
    root = createRoot(container);
    render(<ManagedChildNotice parent="parent" child="child-call-security" />);
    (await until(() => button("Details"), "child notice lacks details"))
      .click();
    await until(
      () => text().includes("Input snapshot") ? true : null,
      "child details did not open",
    );
    check(
      !button("Open as session"),
      "a child's own details must not offer to open itself",
    );
    setActiveSessionId("child-call-security");
    button("Open parent")!.click();
    check(
      getActiveSessionId() === "parent",
      "child notice must open its parent",
    );
    tests.push("child-notice-links-parent");

    // Session tools: calls start off; enabling them stores only the change.
    root.unmount();
    root = createRoot(container);
    render(<SessionToolsSection sessionId="parent" />);
    const allow = await until(
      () =>
        document.querySelector<HTMLInputElement>(
          "input[aria-label='Allow agent calls']",
        ),
      "session tools did not load",
    );
    check(!allow.checked, "agent calls must start off");
    check(
      text().includes("Claude defaults"),
      "inherited defaults must be named",
    );
    allow.click();
    await until(
      () => sessionWrites.length === 1 ? true : null,
      "enabling calls was not saved",
    );
    check(
      JSON.stringify(sessionWrites[0]) ===
        JSON.stringify({ schema: 1, calls: { enabled: true } }),
      `session override must hold only the change: ${
        JSON.stringify(sessionWrites[0])
      }`,
    );
    await until(
      () => button("Use Claude defaults"),
      "an overridden session must offer its defaults",
    );
    await until(
      () => {
        const input = document.querySelector<HTMLInputElement>(
          "input[aria-label='Allow calls to Codex']",
        );
        return input && !input.disabled ? input : null;
      },
      "targets must be editable once calls are on",
    );
    // A Claude session calls other families only: one card, no chooser.
    check(
      document.querySelectorAll("[data-call-target]").length === 1 &&
        document.querySelector("[data-call-target='codex']"),
      "a Claude session must offer exactly the Codex card",
    );
    check(
      !document.querySelector("[aria-label='Default called agent']") &&
        !text().includes("Default agent"),
      "one callable agent needs no default chooser",
    );
    check(
      text().includes("ask Codex for a read-only review"),
      "the switch must name what it starts",
    );
    // The card starts folded; its settings unfold under its header.
    check(
      !document.querySelector("[data-call-target='codex'] [role='radio']"),
      "a target card must start folded",
    );
    (await until(
      () =>
        document.querySelector<HTMLElement>(
          "[data-call-target='codex'] button[aria-label='Codex call settings']",
        ),
      "the Codex card must offer to unfold",
    )).click();
    // The card's presets are choices in the card itself.
    (await until(
      () =>
        document.querySelector<HTMLElement>(
          "[data-call-target='codex'] [role='radio'][aria-label='Astra · Max']",
        ),
      "the Codex card must list its presets",
    )).click();
    await until(
      () => sessionWrites.length === 2 ? true : null,
      "choosing a preset was not saved",
    );
    check(
      JSON.stringify(sessionWrites[1]?.calls?.targets) ===
        JSON.stringify([{ agent: "codex", preset: "astra-max" }]),
      `the preset must be stored on its target: ${
        JSON.stringify(sessionWrites[1])
      }`,
    );
    await until(
      () =>
        document.querySelector(
            "[data-call-target='codex'] [role='radio'][aria-checked='true'][aria-label='Astra · Max']",
          )
          ? true
          : null,
      "the chosen preset must be marked",
    );
    // A finer setting turns the preset into a custom choice of its values.
    const reasoning = await until(
      () => {
        const select = document.querySelector<HTMLElement>(
          "[data-call-target='codex'] [role='combobox'][aria-label='Codex Reasoning']",
        ) ?? [...document.querySelectorAll<HTMLElement>(
          "[data-call-target='codex'] [data-call-target-options] [role='combobox']",
        )].at(-1);
        return select && select.getAttribute("aria-disabled") !== "true"
          ? select
          : null;
      },
      "the Codex card must offer its finer settings",
    );
    check(
      reasoning.textContent?.includes("Max"),
      `custom settings must show the preset's values: ${reasoning.textContent}`,
    );
    reasoning.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true, button: 0 }),
    );
    (await until(
      () =>
        [...document.querySelectorAll<HTMLElement>("[role='option']")].find(
          (node) => node.textContent?.trim() === "High",
        ),
      "the reasoning menu must open",
    )).click();
    await until(
      () => sessionWrites.length === 3 ? true : null,
      "a custom setting was not saved",
    );
    check(
      JSON.stringify(sessionWrites[2]?.calls?.targets) ===
        JSON.stringify([{
          agent: "codex",
          options: { model: "gpt-6-astra", reasoning_effort: "high" },
        }]),
      `a custom choice must store its values: ${JSON.stringify(sessionWrites[2])}`,
    );
    await until(
      () =>
        document.querySelector(
            "[data-call-target='codex'] [role='radio'][aria-checked='true']",
          )
          ? null
          : true,
      "a custom choice selects no preset",
    );
    (await until(
      () => {
        const input = document.querySelector<HTMLInputElement>(
          "input[aria-label='Allow calls to Codex']",
        );
        return input && !input.disabled ? input : null;
      },
      "the target switch must be editable after saving",
    )).click();
    await until(
      () => sessionWrites.length === 4 ? true : null,
      "removing a target was not saved",
    );
    check(
      JSON.stringify(sessionWrites[3]?.calls?.targets) === "[]",
      `removing Codex must leave no target: ${
        JSON.stringify(sessionWrites[3])
      }`,
    );
    (await until(() => {
      const reset = button("Use Claude defaults");
      return reset && !reset.disabled ? reset : null;
    }, "reset must be available after saving")).click();
    await until(
      () => sessionWrites.length === 5 ? true : null,
      "reset was not saved",
    );
    check(
      JSON.stringify(sessionWrites[4]) === JSON.stringify({ schema: 1 }),
      "reset must clear the override",
    );
    check(
      JSON.stringify(overrideFor(toolsDefaults, toolsDefaults)) ===
        JSON.stringify({ schema: 1 }),
      "defaults must produce an empty override",
    );
    const preference = await until(() => {
      const select = document.querySelector<HTMLElement>("[role='combobox'][aria-label='GPT review preference']");
      return select?.getAttribute("aria-disabled") !== "true" ? select : null;
    }, "review preference must be editable while calls are off");
    check(preference.textContent?.includes("Astra → SOL"), "GPT defaults prefer Astra");
    preference.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    (await until(() => [...document.querySelectorAll<HTMLElement>("[role='option']")]
      .find((node) => node.textContent?.trim() === "SOL only"), "SOL-only preference must be available")).click();
    await until(() => sessionWrites.length === 6 ? true : null, "review preference was not saved");
    check(JSON.stringify(sessionWrites[5]?.calls?.review) ===
      JSON.stringify({ gpt: ["sol"] }), "review preference overrides only GPT and excludes unselected models");
    (await until(() => {
      const reset = button("Use Claude defaults");
      return reset && !reset.disabled ? reset : null;
    }, "review override can restore defaults")).click();
    await until(() => sessionWrites.length === 7 ? true : null, "review reset was not saved");
    check(JSON.stringify(sessionWrites[6]) === JSON.stringify({ schema: 1 }), "review reset resumes inheritance");
    tests.push("session-tools-store-only-changes");

    // Agent defaults: one switch writes the whole default for that kind.
    root.unmount();
    root = createRoot(container);
    render(<AgentToolsSettings />);
    const defaultsSwitch = await until(
      () =>
        document.querySelector<HTMLInputElement>(
          "input[aria-label='Allow agent calls']",
        ),
      "agent defaults did not load",
    );
    check(
      document.querySelector("[data-call-target='claude-code']") &&
        !document.querySelector("[data-call-target='codex']"),
      "Codex defaults must offer Claude and never Codex",
    );
    defaultsSwitch.click();
    await until(
      () => agentWrites.length === 1 ? true : null,
      "agent defaults were not saved",
    );
    check(agentWrites[0]?.calls.enabled === true, "defaults must enable calls");
    (await until(() => {
      const reset = button("Reset");
      return reset && !reset.disabled ? reset : null;
    }, "customized defaults must offer reset")).click();
    await until(
      () => agentWrites.length === 2 ? true : null,
      "reset defaults were not saved",
    );
    check(agentWrites[1] === null, "reset must restore built-in defaults");
    tests.push("agent-defaults-save-and-reset");

    // A refused call becomes one actionable prompt; answering removes it.
    root.unmount();
    root = createRoot(container);
    const pending: CallApproval = {
      schema: 1,
      caller: "claude-code",
      reason: "calls_disabled",
      requests: 3,
      agents: ["codex"],
      items: [
        { agent: "codex", purpose: "review", summary: "aspect=security" },
        { agent: "codex", purpose: "review", summary: "aspect=api" },
        { agent: "codex", purpose: "design_review", summary: "aspect=spec" },
      ],
      ttl_ms: 15_000,
    };
    function ApprovalSlot(): React.JSX.Element | null {
      const approval = useCallApproval("parent");
      return approval
        ? <CallApprovalOverlay approval={approval} sessionId="parent" />
        : null;
    }
    render(<ApprovalSlot />);
    check(
      !document.querySelector("[data-call-approval-overlay]"),
      "no prompt before the agent asks",
    );
    receiveCallApproval("parent", pending);
    await until(
      () => document.querySelector("[data-call-approval-overlay]"),
      "a pushed approval must render",
    );
    check(
      text().includes("Claude wants to start 3 calls (Codex)") &&
        text().includes("Agent calls are off for this session.") &&
        text().includes("design review · aspect=spec"),
      `the prompt must name caller, targets and requests: ${text()}`,
    );
    (await until(() => button("Allow these calls"), "allow is missing"))
      .click();
    await until(
      () => decisions.length === 1 ? true : null,
      "allowing was not sent",
    );
    check(decisions[0] === "once", `allow must be once: ${decisions[0]}`);
    await until(
      () =>
        document.querySelector("[data-call-approval-overlay]") ? null : true,
      "an answered prompt must disappear",
    );
    receiveCallApproval("parent", {
      ...pending,
      requests: 1,
      items: [pending.items[0]!],
    });
    (await until(
      () => button("Allow for this session"),
      "session allow is missing",
    ))
      .click();
    await until(
      () => decisions.length === 2 ? true : null,
      "session allow was not sent",
    );
    check(decisions[1] === "session", `expected session: ${decisions[1]}`);
    receiveCallApproval("parent", pending, Date.now() - 20_000);
    await until(
      () =>
        document.querySelector("[data-call-approval-overlay]") ? null : true,
      "an expired prompt must not render",
    );
    tests.push("call-approval-prompt-answers-and-expires");
    return tests;
  } catch (error) {
    throw new Error(`After ${tests.at(-1) ?? "setup"}: ${error instanceof Error ? `${error.message}\n${error.stack}` : String(error)}`);
  } finally {
    root.unmount();
    container.remove();
    globalThis.fetch = originalFetch;
    composerStackExpandedStore().set(null);
  }
}

/** Leave one realistic state rendered for visual inspection (screenshots). */
export async function renderManagedCallsPreview(
  surface: "desktop" | "mobile",
  railOnly = false,
): Promise<void> {
  bindProductSyncPrincipal("call-account-fixture");
  const calls = [
    summary("call-security", "running", "security"),
    summary("call-correctness", "completed", "correctness", {
      configuration: { model: "gpt-6-astra", reasoning_effort: "max" },
      preset: "astra-max",
      started_at_ms: Date.now() - 90_000,
      finished_at_ms: Date.now() - 1_000,
      verdict: "needs-attention",
      finding_count: 2,
    }),
    summary("call-tests", "failed", "test-coverage"),
  ];
  const provider = managementEntryFixture("codex");
  provider.manifest.display.name = "Codex";
  globalThis.fetch = ((input: RequestInfo | URL) => {
    // A focused bootstrap names the client's cached high-water seq
    // (`?since_seq=`, docs/background-tasks.md); route it by its path.
    const url = String(input).replace(/(\/bootstrap)\?since_seq=\d+$/, "$1");
    if (url === "/api/provider-accounts") return Promise.resolve(Response.json({
      dataset: { dataset_id: "calls-fixture", user_id: "call-account-fixture" },
      registry: { revision: 1, accounts: [], defaults: {} }, session_admission: "enabled",
      display_preferences: { scope: "subscriptions", revision: 1, values: { legacy_labels: { codex: "OpenAI · Pika" } } },
    }));
    const bootstrap = childBootstrap(url);
    if (bootstrap) return Promise.resolve(bootstrap);
    if (url === "/api/plugins") {
      return Promise.resolve(
        Response.json({
          providers: [provider],
          authentications: [],
          authentication_executors: [],
        }),
      );
    }
    if (url === "/api/sessions/parent/calls") {
      return Promise.resolve(
        Response.json({ schema: 1, calls, next_before: null }),
      );
    }
    const detail = /^\/api\/sessions\/parent\/calls\/([^/]+)$/.exec(url);
    const found = detail && calls.find((call) => call.call_id === detail[1]);
    return Promise.resolve(
      found
        ? Response.json({
          schema: 1,
          call: { ...found, result: findingsResult },
        })
        : new Response("not found", { status: 404 }),
    );
  }) as typeof fetch;
  resetProviderCatalog();
  const container = document.createElement("div");
  container.style.cssText = surface === "desktop"
    ? "width: 520px; margin: 16px;"
    : "position: fixed; left: 0; right: 0; bottom: 0; padding: 8px;";
  document.body.append(container);
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <BrowserProductTheme>
        <SurfaceProvider>
          <CssBaseline />
          <CallApprovalOverlay
            sessionId="parent"
            approval={{
              schema: 1,
              caller: "claude-code",
              reason: "calls_disabled",
              requests: 2,
              agents: ["codex"],
              items: [
                {
                  agent: "codex",
                  purpose: "review",
                  summary: "aspect=security group=T-42 round=1",
                },
                {
                  agent: "codex",
                  purpose: "design_review",
                  summary: "aspect=spec group=T-42 round=1",
                },
              ],
              ttl_ms: 15_000,
            }}
          />
          <Box sx={{ height: 8 }} />
          <ManagedCallsDock
            sessionId="parent"
            desktop={surface === "desktop"}
          />
        </SurfaceProvider>
      </BrowserProductTheme>,
    )
  );
  const button = (label: string) =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].find((node) =>
      node.getAttribute("aria-label") === label ||
      node.getAttribute("aria-label")?.includes(label)
    );
  if (surface === "desktop") {
    (await until(() => button("Expand calls"), "desktop header")).click();
    const rows = await until(
      () => {
        const found = document.querySelectorAll<HTMLElement>(
          "[data-desktop-item^='call-']",
        );
        return found.length === 3 ? found : null;
      },
      "desktop rows",
    );
    rows[1]!.click();
  } else {
    if (railOnly) {
      await until(() => button("Open calls"), "mobile preview rail");
      return;
    }
    (await until(() => button("Open calls"), "mobile header")).click();
    (await until(() => button("correctness"), "mobile row")).click();
    await until(
      () => document.querySelector("[data-call-transcript]"),
      "preview transcript",
    );
    return;
  }
  await until(
    () =>
      (document.body.textContent ?? "").includes("Retry duplicates the charge")
        ? true
        : null,
    "preview result",
  );
}

/** A Claude session's Tools with calls on and a Codex preset chosen. */
export function renderAgentToolsPreview(unfold: unknown = false): void {
  const override: SessionToolsOverride = {
    schema: 1,
    calls: { enabled: true, targets: [{ agent: "codex", preset: "astra-max" }] },
  };
  globalThis.fetch = ((input: RequestInfo | URL) =>
    Promise.resolve(
      String(input) === "/api/sessions/parent/tools"
        ? Response.json({
          session_id: "parent",
          agent: "claude-code",
          defaults: toolsDefaults,
          defaults_customized: false,
          override,
          effective: overlay(toolsDefaults, override),
          catalog: {
            call_targets: [{
              agent: "codex",
              presets: [
                {
                  id: "astra-max",
                  name: "Astra · Max",
                  detail: "GPT-6 Astra · Max reasoning",
                  is_default: false,
                  values: { model: "gpt-6-astra", reasoning_effort: "max" },
                },
                {
                  id: "astra-high",
                  name: "Astra · High",
                  detail: "GPT-6 Astra · High reasoning",
                  is_default: true,
                  values: { model: "gpt-6-astra", reasoning_effort: "high" },
                },
              ],
              options: toolsCatalog.call_targets[0]!.options,
            }, ...toolsCatalog.call_targets.filter((target) =>
              target.agent !== "codex"
            )],
          },
        })
        : new Response("not found", { status: 404 }),
    )) as typeof fetch;
  const container = document.createElement("div");
  container.style.cssText = "max-width: 560px; margin: 0 auto; padding: 16px;";
  document.body.append(container);
  createRoot(container).render(
    <BrowserProductTheme>
      <SurfaceProvider>
        <SessionToolsSection sessionId="parent" />
      </SurfaceProvider>
    </BrowserProductTheme>,
  );
  if (unfold === true) {
    const open = (): void => {
      const header = document.querySelector<HTMLElement>(
        "button[aria-label='Codex call settings']",
      );
      if (header) header.click();
      else setTimeout(open, 50);
    };
    open();
  }
}
