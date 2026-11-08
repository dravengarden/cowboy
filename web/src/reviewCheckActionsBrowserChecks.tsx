import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { BrowserProductTheme } from "./browserProductTheme";
import { ReviewCheckStatus } from "./mobile/review/ReviewCheckStatus";
import { refreshReviewChecks } from "./review/reviewChecksStore";
import type { CodeChecks } from "./review/codeApi";

export async function reviewCheckActionsBrowserChecks(): Promise<string> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const original = globalThis.fetch;
  const requests: unknown[] = [];
  const checks: CodeChecks = {
    apiVersion: 1,
    epoch: "fixture-epoch",
    allowed: true,
    runnable: true,
    declared: [{ id: "lint", title: "Lint", format: "lines", timeoutSeconds: 30 }],
    runs: [],
  };
  const until = async (predicate: () => boolean) => {
    for (let i = 0; i < 300; i++) {
      if (predicate()) return;
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    throw new Error("Check action fixture timed out");
  };
  globalThis.fetch = (async (url, init) => {
    if (String(url) !== "/api/code/sessions/check-fixture/checks") return original(url, init);
    if (init?.method !== "POST") return Response.json(checks);
    requests.push(JSON.parse(String(init.body)));
    if (requests.length === 1) throw new TypeError("lost reply");
    return Response.json({ status: "existing" });
  }) as typeof fetch;
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <ReviewCheckStatus sessionId="check-fixture" path="a.ts" />
        </BrowserProductTheme>,
      ),
    );
    await until(() => !!container.querySelector("button"));
    container.querySelector("button")!.click();
    await until(() => !!container.textContent?.includes("outcome is unknown"));
    checks.runs = [
      {
        runId: "run",
        check: "lint",
        state: "passed",
        startedAtMs: Date.now(),
        diagnostics: [],
        truncated: false,
        outputTruncated: false,
      },
    ];
    refreshReviewChecks("check-fixture");
    await until(() => !!container.querySelector("[data-review-check-status]"));
    if (!container.textContent?.includes("outcome is unknown"))
      throw new Error("Result polling discarded uncertain request state");
    container.querySelector("button")!.click();
    await until(
      () => requests.length === 2 && !container.textContent?.includes("outcome is unknown"),
    );
    if (JSON.stringify(requests[0]) !== JSON.stringify(requests[1]))
      throw new Error("Retry changed operation or epoch");
    return "real check action retains uncertain request identity when the first run status appears";
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    globalThis.fetch = original;
  }
}
