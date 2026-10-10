/** Actual Calls dock and child notice, synthetic HTTP in an isolated browser. */
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { ManagedCallsDock, ManagedChildNotice } from "./ManagedCallsDock";
import { getActiveSessionId, setActiveSessionId } from "./controlPlane";
import { composerStackExpandedStore } from "./composerStackAccordion";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import type { ManagedCallSummary } from "./managedCalls";
import { AgentToolsSettings, SessionToolsSection } from "./AgentToolsPanel";
import { CallApprovalOverlay } from "./CallApprovalOverlay";
import {
  type CallApproval,
  receiveCallApproval,
  useCallApproval,
} from "./callApproval";
import {
  type AgentTools,
  overrideFor,
  type SessionToolsOverride,
} from "./agentTools";

const toolsDefaults: AgentTools = {
  schema: 1,
  matrix: { tools: true, recall: true },
  calls: {
    enabled: false,
    targets: [{ agent: "claude-code" }, { agent: "codex" }],
    default: "auto",
    max_concurrent: 4,
    max_per_session: 64,
  },
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
      }],
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
    matrix: { ...defaults.matrix, ...override.matrix },
    calls: { ...defaults.calls, ...override.calls },
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
        update(
          2,
          "agent_message_chunk",
          "The retry path in `charge.go` resubmits after the provider accepted " +
            "the request, so a charge can be **duplicated**.\n\n" +
            "The outbound client also has no deadline.",
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
  const originalFetch = globalThis.fetch;
  const tests: string[] = [];
  let calls: ManagedCallSummary[] = [];
  const cancels: string[] = [];
  const cancelCount = (): number => cancels.length;
  let sessionOverride: SessionToolsOverride = { schema: 1 };
  const sessionWrites: SessionToolsOverride[] = [];
  let agentDefaults: AgentTools = toolsDefaults;
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
    const url = String(input);
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
          customized: agentDefaults !== toolsDefaults,
        }],
        catalog: toolsCatalog,
      }));
    }
    if (url === "/api/agent-tools/codex" && init?.method === "PUT") {
      const body = JSON.parse(String(init.body)) as {
        settings: AgentTools | null;
      };
      agentWrites.push(body.settings);
      agentDefaults = body.settings ?? toolsDefaults;
      return Promise.resolve(Response.json({
        schema: 1,
        agent: "codex",
        settings: agentDefaults,
        customized: body.settings !== null,
      }));
    }
    if (url === "/api/sessions/parent/calls") {
      return Promise.resolve(
        Response.json({ schema: 1, calls, next_before: null }),
      );
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
  const render = (node: React.ReactNode) =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <SurfaceProvider>{node}</SurfaceProvider>
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
    composerStackExpandedStore().set(null);
    render(<ManagedCallsDock sessionId="parent" desktop />);
    await wait(150);
    check(container.textContent === "", "dock without calls must not render");
    tests.push("hidden-without-calls");

    root.unmount();
    root = createRoot(container);
    calls = [
      summary("call-security", "running", "security"),
      summary("call-correctness", "completed", "correctness", {
        verdict: "needs-attention",
        finding_count: 2,
      }),
      summary("call-tests", "failed", "test-coverage"),
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

    button("Expand calls")!.click();
    await until(() => button("Collapse calls"), "calls did not expand");
    const rows = document.querySelectorAll("[data-desktop-item^='call-']");
    check(rows.length === 3, "every call must be a keyboard item");
    check(
      text().includes("OVH → Hawk"),
      "runtime and execution placement missing",
    );
    check(text().includes("PR 7961 review"), "caller group label missing");
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
    check(
      text().includes("internal/billing/charge.go:42"),
      "finding location missing",
    );
    check(
      text().includes("recorded by the calling workflow"),
      "disposition ownership note missing",
    );
    check(text().includes("Input snapshot"), "snapshot identity missing");
    tests.push("desktop-modal-shows-transcript-and-details");

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
    open.click();
    const row = await until(
      () =>
        [...document.querySelectorAll<HTMLButtonElement>("button")].find((
          node,
        ) => node.getAttribute("aria-label")?.includes("correctness")),
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
    const backButton = (): HTMLElement | null =>
      document.querySelector<HTMLElement>(
        "[data-mobile-sheet-footer-shield] button[aria-label='Back']",
      );
    check(backButton(), "a nested level must offer Back in the island");
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
    const codexAllowed = await until(
      () => {
        const input = document.querySelector<HTMLInputElement>(
          "input[aria-label='Allow calls to Codex']",
        );
        return input && !input.disabled ? input : null;
      },
      "targets must be editable once calls are on",
    );
    codexAllowed.click();
    await until(
      () => sessionWrites.length === 2 ? true : null,
      "removing a target was not saved",
    );
    check(
      JSON.stringify(sessionWrites[1]?.calls?.targets) ===
        JSON.stringify([{ agent: "claude-code" }]),
      `removing Codex must keep only Claude: ${
        JSON.stringify(sessionWrites[1])
      }`,
    );
    (await until(() => {
      const reset = button("Use Claude defaults");
      return reset && !reset.disabled ? reset : null;
    }, "reset must be available after saving")).click();
    await until(
      () => sessionWrites.length === 3 ? true : null,
      "reset was not saved",
    );
    check(
      JSON.stringify(sessionWrites[2]) === JSON.stringify({ schema: 1 }),
      "reset must clear the override",
    );
    check(
      JSON.stringify(overrideFor(toolsDefaults, toolsDefaults)) ===
        JSON.stringify({ schema: 1 }),
      "defaults must produce an empty override",
    );
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
): Promise<void> {
  const calls = [
    summary("call-security", "running", "security"),
    summary("call-correctness", "completed", "correctness", {
      verdict: "needs-attention",
      finding_count: 2,
    }),
    summary("call-tests", "failed", "test-coverage"),
  ];
  const provider = managementEntryFixture("codex");
  provider.manifest.display.name = "Codex";
  globalThis.fetch = ((input: RequestInfo | URL) => {
    const url = String(input);
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
