/** Widget apps (docs/widget-apps.md §9): the real sandbox frame, runtime
 *  Worker and QuickJS VM run agent-style TSX in a disposable browser context.
 *  Positive rendering and interaction, then hostile apps that must stop
 *  within their budgets while this page stays responsive. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import AppNode from "./widgets/apps/AppNode";

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
  root.unmount();
  container.remove();
  void sent;
  return passed;
}
