/** Actual Git review recovery after a failed load. Synthetic HTTP, no account. */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createTheme, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile.tsx";
import { ReviewChanges } from "./mobile/review/ReviewChanges.tsx";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function until(predicate: () => boolean, label: string) {
  for (let n = 0; n < 1_500; n++) {
    if (predicate()) return;
    await new Promise<void>((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`Review recovery fixture timed out: ${label}`);
}
const sleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
const noop = () => undefined;
const CHANGES = "/api/code/sessions/workspace%3A%3Afixture/changes";

export async function runReviewRecoveryBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  let status = 502;
  let requests = 0;
  const originalFetch = globalThis.fetch;
  globalThis.fetch = (input: RequestInfo | URL) => {
    const url = new URL(String(input), location.href);
    if (url.pathname !== CHANGES) {
      return Promise.resolve(new Response("not found", { status: 404 }));
    }
    requests += 1;
    if (status !== 200) {
      return Promise.resolve(new Response("unavailable", { status }));
    }
    return Promise.resolve(
      new Response(
        JSON.stringify({
          apiVersion: 1,
          revision: "r1",
          head: "abcdef0",
          truncated: false,
          changes: [{
            path: "src/recovered.ts",
            status: "modified",
            staged: false,
            unstaged: true,
          }],
        }),
        { headers: { "Content-Type": "application/json" } },
      ),
    );
  };
  const setVisibility = (state: "visible" | "hidden") => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => state,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  };
  const container = document.createElement("div");
  container.style.cssText =
    "width: 420px; height: 600px; display: flex; flex-direction: column;";
  document.body.append(container);
  const root = createRoot(container);
  const mount = () =>
    root.render(
      <StrictMode>
        <ThemeProvider theme={createTheme()}>
          <SurfaceProvider>
            <ReviewChanges
              sessionId="workspace::fixture"
              onOpenDiff={noop}
              reviewed={new Set<string>()}
              onRevision={noop}
            />
          </SurfaceProvider>
        </ThemeProvider>
      </StrictMode>,
    );
  const text = () => container.textContent ?? "";
  const failed = () => text().includes("Git changes are unavailable");
  const listed = () => text().includes("recovered.ts");

  try {
    // The runner's page is foregrounded; pin it so the schedule is the subject.
    setVisibility("visible");
    mount();
    await until(failed, "unavailable alert");
    check(!listed(), "changes listed while the Machine was away");
    tests.push(
      "a load that fails while the Machine is away surfaces the unavailable alert",
    );

    status = 200;
    const untouched = requests;
    await until(listed, "unattended recovery");
    check(!failed(), "the alert outlived the failure");
    check(requests > untouched, "recovery reused the failed response");
    tests.push(
      "the panel loads the changes by itself once the Machine answers again, with no control pressed",
    );

    status = 404;
    root.unmount();
    const missing = createRoot(container);
    missing.render(
      <StrictMode>
        <ThemeProvider theme={createTheme()}>
          <SurfaceProvider>
            <ReviewChanges
              sessionId="workspace::fixture"
              onOpenDiff={noop}
              reviewed={new Set<string>()}
              onRevision={noop}
            />
          </SurfaceProvider>
        </ThemeProvider>
      </StrictMode>,
    );
    await until(failed, "durable failure alert");
    await sleep(200);
    const settled = requests;
    await sleep(1_500);
    check(requests === settled, "a durable answer was retried anyway");
    tests.push(
      "a gone session is not retried behind the user's back; Retry stays the control",
    );

    setVisibility("hidden");
    status = 502;
    missing.unmount();
    const hidden = createRoot(container);
    hidden.render(
      <StrictMode>
        <ThemeProvider theme={createTheme()}>
          <SurfaceProvider>
            <ReviewChanges
              sessionId="workspace::fixture"
              onOpenDiff={noop}
              reviewed={new Set<string>()}
              onRevision={noop}
            />
          </SurfaceProvider>
        </ThemeProvider>
      </StrictMode>,
    );
    await until(failed, "hidden page alert");
    const pocketed = requests;
    await sleep(1_500);
    check(requests === pocketed, "a hidden page polled the Machine");
    status = 200;
    setVisibility("visible");
    await until(listed, "foreground recovery");
    tests.push(
      "a hidden page schedules nothing and recovers the moment it is looked at again",
    );
    hidden.unmount();
  } finally {
    globalThis.fetch = originalFetch;
    Reflect.deleteProperty(document, "visibilityState");
    container.remove();
  }
  return tests;
}
