/** Full-DOM apps in the product (docs/widget-apps-dom.md §5, §9 step 3): the
 *  real `dom_app` node inside `WidgetView`, the way a transcript row shows it.
 *  Run starts the frame, the app's send becomes the product confirmation with
 *  its exact text and a `dom_app:` identity, Stop removes the frame, a later
 *  Run takes the page's single full-app ownership, and an app out of view is
 *  stopped after its grace. Runs only on an admitted browser. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import WidgetView from "./widgets/WidgetView";
import { parseWidgetValue } from "./widgets/widgetDocument";
import { setAppMessageTransport } from "./widgets/apps/appMessages";
import { SurfaceContext, type SurfaceProfile } from "./surface/SurfaceProfile";

const DESKTOP: SurfaceProfile = {
  kind: "desktop",
  input: "keyboard-pointer",
  touchCapable: false,
  finePointer: true,
  hover: true,
};

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

async function waitFor<T>(
  probe: () => T | null | undefined | false,
  label: string,
  ms = 15_000,
): Promise<T> {
  const until = performance.now() + ms;
  while (performance.now() < until) {
    const found = probe();
    if (found) return found;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error(`timed out: ${label}`);
}

// The app draws with WebGL and asks to send once it has drawn.
const APP = `
import { useEffect, useRef } from "react";
import * as THREE from "three";
import { useSend } from "@cowboy/dom";
export default function App({ label }: { label: string }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const send = useSend();
  useEffect(() => {
    const r = new THREE.WebGLRenderer({ canvas: ref.current! });
    const scene = new THREE.Scene(); scene.background = new THREE.Color(0x22aa44);
    r.render(scene, new THREE.PerspectiveCamera());
    send("dom app says " + label);
  }, []);
  return <div><p>{label}</p><canvas ref={ref} width={64} height={64} /></div>;
}`;

function documentFor(id: string, label: string) {
  const parsed = parseWidgetValue({
    schema: "cowboy.widget/1",
    title: `Full app ${id}`,
    root: {
      type: "dom_app",
      id,
      title: `Cube ${id}`,
      fallback: "A cube.",
      code: APP,
      props: { label },
    },
  });
  check(parsed.ok, "the dom_app widget validates");
  return parsed.document;
}

const runButton = (scope: ParentNode, id: string): HTMLButtonElement | undefined =>
  [...scope.querySelectorAll("button")].find((button) =>
    button.textContent?.includes(`Run “Cube ${id}”`),
  );

export async function runDomAppsProductConformance(): Promise<string[]> {
  const passed: string[] = [];
  const calls: Record<string, unknown>[] = [];
  setAppMessageTransport(async (_url, body) => {
    calls.push(body as Record<string, unknown>);
    return { status: 200, body: { status: "sent" } };
  });
  const container = document.createElement("div");
  container.style.width = "640px";
  document.body.append(container);
  const root = createRoot(container);
  const show = (ids: string[], offset = 0): void =>
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider value={DESKTOP}>
          <BrowserProductTheme>
            <CssBaseline />
            {ids.map((id, index) => (
              <div
                key={id}
                data-row={id}
                style={index === 0 && offset ? { marginTop: offset } : undefined}
              >
                <WidgetView
                  document={documentFor(id, `hello ${id}`)}
                  verified
                  interaction={{
                    sessionId: "s",
                    widgetId: `w-${id}`,
                    revision: 1,
                    closed: false,
                    decided: {},
                  }}
                />
              </div>
            ))}
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      ),
    );
  const frames = (): number => container.querySelectorAll("iframe").length;
  try {
    show(["a", "b"]);
    check(frames() === 0, "a full app never starts by itself");
    (await waitFor(() => runButton(container, "a"), "Run for app a")).click();
    const frame = await waitFor(() => container.querySelector("iframe"), "app a's frame");
    check(frame.getAttribute("sandbox") === "allow-scripts", "the frame is opaque");
    const quoted = await waitFor(
      () => document.querySelector("[data-app-message-text]"),
      "app a's send reaches the confirmation",
      30_000,
    );
    check(quoted.textContent === "dom app says hello a", "the exact app text is shown");
    check(calls.length === 0, "nothing leaves before the person confirms");
    document.querySelector<HTMLElement>("[data-app-message-send]")?.click();
    await waitFor(() => calls.length === 1, "the confirmed send");
    const sent = calls[0] ?? {};
    check(sent["text"] === "dom app says hello a", "the send carries the exact text");
    check(
      typeof sent["code_sha256"] === "string" && /^dom_app:[0-9a-f]{64}$/.test(sent["code_sha256"]),
      `the send carries a dom_app identity (got ${JSON.stringify(sent).slice(0, 300)})`,
    );
    passed.push("Run starts the app; its send is confirmed with a dom_app identity");

    (await waitFor(() => runButton(container, "b"), "Run for app b")).click();
    await waitFor(
      () => frames() === 1 && container.querySelector("[data-row='b'] iframe"),
      "app b replaces app a",
    );
    check(runButton(container, "a"), "app a offers Run again");
    passed.push("one full app runs per page; a later Run stops the earlier one");

    const stop = [...container.querySelectorAll<HTMLButtonElement>("[data-row='b'] button")].find(
      (button) => button.textContent?.includes("Stop"),
    );
    check(stop, "Stop for app b");
    stop.click();
    await waitFor(() => frames() === 0 && runButton(container, "b"), "Stop removes the frame");
    passed.push("Stop removes the frame");

    // Out of view: the first row is pushed far below the viewport.
    show(["a", "b"], 20_000);
    (await waitFor(() => runButton(container, "a"), "Run for app a again")).click();
    await waitFor(() => frames() === 1, "app a starts out of view");
    await waitFor(
      () => frames() === 0 && container.textContent?.includes("out of view"),
      "an app out of view is stopped",
      6_000,
    );
    passed.push("an app out of view is stopped after its grace");
    return passed;
  } finally {
    setAppMessageTransport(null);
    root.unmount();
    container.remove();
  }
}
