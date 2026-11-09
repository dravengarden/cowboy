/** Widget apps (docs/widget-apps.md §9): the real sandbox frame, runtime
 *  Worker and QuickJS VM run agent-style TSX in a disposable browser context.
 *  Positive rendering and interaction, then hostile apps that must stop
 *  within their budgets while this page stays responsive. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import AppNode from "./widgets/apps/AppNode";
import WidgetView from "./widgets/WidgetView";
import { parseWidgetValue } from "./widgets/widgetDocument";
import { setAppMessageTransport } from "./widgets/apps/appMessages";
import { SurfaceContext, type SurfaceProfile } from "./surface/SurfaceProfile";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

async function waitFor<T>(
  probe: () => T | null | undefined | false,
  label: string,
  ms = 8000,
): Promise<T> {
  const until = performance.now() + ms;
  while (performance.now() < until) {
    const found = probe();
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`timed out: ${label}`);
}

const DASHBOARD = `
import * as React from "react";
import { Stack, Heading, Text, Button, TextField, Table, Chart } from "@cowboy/ui";
type Row = { path: string; size: number };
export default function App({ rows }: { rows: Row[] }) {
  const [count, setCount] = React.useState(0);
  const [note, setNote] = React.useState("");
  return (
    <Stack gap="sm">
      <Heading>Disk usage</Heading>
      <Text>{"Clicked " + count + " times"}</Text>
      <Button onPress={() => setCount((n) => n + 1)}>Count</Button>
      <TextField label="Note" value={note} onChange={(e: { value: string }) => setNote(e.value)} />
      <Text>{"Echo: " + note}</Text>
      <Table columns={["Path", "Size"]} rows={rows.map((r) => [r.path, r.size])} />
      <Chart type="bar" data={rows.map((r) => ({ y: r.size }))} series={[{ key: "y", label: "Size" }]} />
    </Stack>
  );
}`;

const app = (body: string): string =>
  `import * as React from "react";\nimport { Text, Stack } from "@cowboy/ui";\n${body}`;

export async function runWidgetAppsBrowserConformance(): Promise<string[]> {
  const passed: string[] = [];
  const errors: string[] = [];
  const originalError = console.error;
  console.error = (...args: unknown[]): void => {
    errors.push(args.map(String).join(" ").slice(0, 300));
    originalError(...args);
  };
  addEventListener("error", (event) => errors.push(`error: ${event.message}`));
  addEventListener("unhandledrejection", (event) =>
    errors.push(`rejection: ${String(event.reason)}`),
  );
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const sent: string[] = [];
  const render = (code: string, props: unknown = {}): void =>
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <CssBaseline />
          <Box sx={{ width: 760, p: 1 }}>
            <AppNode
              key={code}
              code={code}
              props={props}
              title="fixture"
              onSend={(t): void => void sent.push(t)}
            />
          </Box>
        </BrowserProductTheme>,
      ),
    );
  const text = (): string => container.textContent ?? "";

  render(DASHBOARD, {
    rows: [
      { path: "target", size: 900 },
      { path: "logs", size: 300 },
    ],
  });
  await waitFor(
    () => container.querySelector("[data-widget-app] table"),
    "dashboard renders",
  ).catch((error: Error) => {
    throw new Error(
      `${error.message} — page shows: ${text().slice(0, 300)} — html: ${container.innerHTML.slice(0, 600)} — frames: ${document.querySelectorAll("iframe").length} — errors: ${errors.join(" | ").slice(0, 1500)}`,
    );
  });
  check(text().includes("Disk usage") && text().includes("target"), "app content renders");
  check(container.querySelector("[data-widget-app] svg rect"), "the chart draws bars");
  passed.push("TSX app compiles in QuickJS and renders with Cowboy components");

  const button = [...container.querySelectorAll("button")].find((b) => b.textContent === "Count");
  check(button, "the app's button renders");
  button.click();
  button.click();
  await waitFor(() => text().includes("Clicked 2 times"), "clicks round-trip");
  const input = container.querySelector<HTMLInputElement>("[data-widget-app] input");
  check(input, "the app's text field renders");
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  setter?.call(input, "hello");
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await waitFor(() => text().includes("Echo: hello"), "typing round-trips");
  passed.push("events and typed input round-trip through app state");

  render(
    app(
      `export default function A() { return <Text>{[typeof fetch, typeof Worker, typeof XMLHttpRequest, typeof WebSocket, typeof postMessage, typeof document].join(",")}</Text>; }`,
    ),
  );
  await waitFor(
    () => text().includes("undefined,undefined,undefined,undefined,undefined,undefined"),
    "no web APIs in the VM",
  );
  passed.push("the VM has no fetch, Worker, XHR, WebSocket, postMessage or DOM");

  const stops = async (code: string, reason: string, label: string): Promise<void> => {
    let worstLag = 0;
    let last = performance.now();
    const ticker = setInterval(() => {
      const now = performance.now();
      worstLag = Math.max(worstLag, now - last - 50);
      last = now;
    }, 50);
    render(code);
    try {
      await waitFor(() => text().includes(reason), label, 10_000).catch((error: Error) => {
        throw new Error(
          `${error.message} — page shows: ${text().slice(0, 300)} — errors: ${errors.join(" | ").slice(0, 600)}`,
        );
      });
    } finally {
      clearInterval(ticker);
    }
    check(
      worstLag < 250,
      `${label}: the page stayed responsive (worst lag ${Math.round(worstLag)} ms)`,
    );
    passed.push(label);
  };
  await stops(
    app("export default function A() { while (true) {} }"),
    "ran too long",
    "an infinite render loop is stopped",
  );
  await stops(
    app(
      "const keep: ArrayBuffer[] = [];\nexport default function A() { for (;;) keep.push(new ArrayBuffer(4 << 20)); }",
    ),
    "used too much memory",
    "an allocation loop is stopped by the memory limit",
  );
  await stops(
    app('import x from "fs";\nexport default function A() { return <Text>{String(x)}</Text>; }'),
    "is not available",
    "a non-allowlisted import fails",
  );
  await stops(
    app(
      "export default function A() { return <Stack>{Array.from({ length: 3000 }, (_, i) => <Text key={i}>x</Text>)}</Stack>; }",
    ),
    "a render update was too large",
    "an oversized render update is refused",
  );
  await stops(
    app(
      "export default function A() {\n" +
        "  const [n, setN] = React.useState(0);\n" +
        "  React.useEffect(() => { setTimeout(() => setN(n + 500)); }, [n]);\n" +
        "  return <Stack>{Array.from({ length: n }, (_, i) => <Text key={i}>x</Text>)}</Stack>;\n" +
        "}",
    ),
    "too many components",
    "a tree that keeps growing is stopped at the node budget",
  );
  passed.push(await sendChecks(container));
  root.unmount();
  container.remove();
  void sent;
  return passed;
}

const DESKTOP: SurfaceProfile = { kind: "desktop", input: "keyboard-pointer", touchCapable: false, finePointer: true, hover: true };

/** send(text) → product confirm showing the app's exact text → confirmed
 *  send with its digest and occurrence (docs/widget-apps.md §7). */
async function sendChecks(container: HTMLElement): Promise<string> {
  const calls: Record<string, unknown>[] = [];
  setAppMessageTransport(async (_url, body) => {
    const request = body as Record<string, unknown>;
    calls.push(request);
    return request["message_sha256"]
      ? { status: 200, body: { status: "sent" } }
      : { status: 200, body: { message: request["text"], message_sha256: "digest-1" } };
  });
  const parsed = parseWidgetValue({
    schema: "cowboy.widget/1",
    title: "Cleanup",
    root: {
      type: "app",
      id: "cleanup",
      title: "Cleanup app",
      fallback: "Cleanup",
      code: app(
        'import { Button, useSend } from "@cowboy/ui";\nexport default function A() { const send = useSend(); return <Button onPress={() => send("Please remove target/")}>Ask</Button>; }',
      ),
    },
  });
  check(parsed.ok, "the app widget validates");
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <SurfaceContext.Provider value={DESKTOP}>
        <BrowserProductTheme>
          <CssBaseline />
          <WidgetView
            document={parsed.document}
            verified
            interaction={{ sessionId: "s", widgetId: "w", revision: 1, closed: false, decided: {} }}
          />
        </BrowserProductTheme>
      </SurfaceContext.Provider>,
    ),
  );
  try {
    const ask = await waitFor(
      () => [...container.querySelectorAll("button")].find((b) => b.textContent === "Ask"),
      "the app's Ask button",
    );
    ask.click();
    const quoted = await waitFor(() => document.querySelector("[data-app-message-text]"), "the confirm shows the text");
    check(quoted.textContent === "Please remove target/", "the exact app text is shown");
    check(document.querySelector("[data-app-message-confirm]")?.textContent?.includes("Written by the app"), "the text is attributed to the app");
    check(calls.length === 1 && !calls[0]?.["message_sha256"], "nothing is sent before the person confirms");
    document.querySelector<HTMLElement>("[data-app-message-send]")?.click();
    await waitFor(() => calls.length === 2, "the confirmed send");
    check(calls[1]?.["message_sha256"] === "digest-1" && calls[1]?.["occurrence"] === calls[0]?.["occurrence"], "the send carries the previewed digest and occurrence");
    await waitFor(() => !document.querySelector("[data-app-message-confirm]"), "the confirm closes");
    return "an app's message is sent only after the person confirms its exact text";
  } finally {
    setAppMessageTransport(null);
    root.unmount();
  }
}
