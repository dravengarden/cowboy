/** Actual Calls dock and child notice, synthetic HTTP in an isolated browser. */
import { createRoot, type Root } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { ManagedCallsDock, ManagedChildNotice } from "./ManagedCallsDock";
import { getActiveSessionId, setActiveSessionId } from "./controlPlane";
import { composerStackExpandedStore } from "./composerStackAccordion";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import type { ManagedCallSummary } from "./managedCalls";

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
  const provider = managementEntryFixture("codex");
  provider.manifest.display.name = "Codex";
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
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
    await until(
      () => text().includes("Retry duplicates the charge") ? true : null,
      "full result not loaded",
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
    tests.push("desktop-detail-shows-review-result");

    (rows[0] as HTMLElement).click();
    await until(
      () => button("Stop call"),
      "stop action missing for a running call",
    );
    button("Stop call")!.click();
    const confirm = await until(
      () =>
        [
          ...document.querySelectorAll<HTMLButtonElement>(
            "[role='dialog'] button",
          ),
        ].find((node) => node.textContent?.trim() === "Stop call"),
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
    button("Open conversation")!.click();
    check(
      getActiveSessionId() === "child-call-security",
      "open conversation must select the child",
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
    await until(
      () => button("Back to calls"),
      "mobile detail missing back action",
    );
    await until(
      () => text().includes("Retry duplicates the charge") ? true : null,
      "mobile detail result missing",
    );
    button("Back to calls")!.click();
    await until(
      () => button("Back to calls") ? null : true,
      "back did not return to the list",
    );
    tests.push("mobile-page-drills-in-and-back");

    root.unmount();
    root = createRoot(container);
    render(<ManagedChildNotice parent="parent" />);
    setActiveSessionId("child-call-security");
    button("Open parent")!.click();
    check(
      getActiveSessionId() === "parent",
      "child notice must open its parent",
    );
    tests.push("child-notice-links-parent");
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
  }
  await until(
    () =>
      (document.body.textContent ?? "").includes("Retry duplicates the charge")
        ? true
        : null,
    "preview result",
  );
}
