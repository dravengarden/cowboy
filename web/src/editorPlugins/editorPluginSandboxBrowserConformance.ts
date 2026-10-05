import type { EditorPort } from "../editorExtensions/contract";
import { EDITOR_PLUGIN_PACKAGE_FORMAT, editorPluginDigest } from "./manifest";
import { createEditorPluginHost } from "./registry";
import { createSandboxTransport } from "./sandbox";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

/** The real iframe + Worker sandbox in this browser, without the App: CSP,
 * opaque origin, version-bound edits, hang termination and cleanup. */
export async function runEditorPluginSandboxBrowserConformance(): Promise<
  string[]
> {
  const results: string[] = [];
  let stored: unknown = null;
  const host = createEditorPluginHost({
    persistence: {
      load: () => Promise.resolve(stored),
      save: (value) => {
        stored = structuredClone(value);
        return Promise.resolve();
      },
    },
    sandbox: createSandboxTransport,
    notice: () => {},
    timeouts: { start: 5000, command: 1500, panel: 1500, unload: 200 },
  });
  const manifest = {
    id: "probe",
    name: "Probe",
    version: "1.0.0",
    description: "Sandbox probe",
    author: "Conformance",
    api: { major: 1, minor: 0 },
    permissions: ["editor:read", "editor:write"],
    contexts: ["document"],
    surfaces: ["desktop"],
    settings: [],
  } as const;
  const main = `definePlugin({ async onload(ctx) {
    const results = [];
    let realFetch;
    for (let o = self; o && !realFetch; o = Object.getPrototypeOf(o)) {
      const d = Object.getOwnPropertyDescriptor(o, "fetch");
      if (d && typeof d.value === "function") realFetch = d.value;
    }
    self.addEventListener("securitypolicyviolation", (e) => { self.__csp = e.effectiveDirective || e.violatedDirective; });
    try { await realFetch.call(self, ${
    JSON.stringify(`${location.origin}/report/probe`)
  }); results.push("fetch-open"); }
    catch { results.push("fetch-blocked"); }
    await new Promise((r) => setTimeout(r, 50));
    results.push(self.origin === "null" ? "opaque" : "origin:" + self.origin);
    results.push("csp:" + (self.__csp || "none"));
    ctx.addCommand({ id: "report", title: results.join(","), run() {} });
    ctx.addCommand({ id: "upper", title: "Upper", async run(editor) {
      await editor.replaceSelection(editor.read().text.toUpperCase());
    }});
    ctx.addCommand({ id: "spin", title: "Spin", run() { for (;;) {} } });
  }});`;
  const source = JSON.stringify({
    format: EDITOR_PLUGIN_PACKAGE_FORMAT,
    manifest,
    main,
    digest: await editorPluginDigest(manifest, main),
  });
  const installed = await host.install(await host.inspect(source));
  check(installed.ok, `install: ${installed.message}`);
  const view = host.getSnapshot().plugins[0]!;
  const report = view.commands.find((c) => c.id === "report")!.title;
  check(
    report === "fetch-blocked,opaque,csp:connect-src",
    `isolation: ${report}`,
  );
  results.push(`Sandbox isolation: ${report}`);

  let text = "abc";
  let revision = 0;
  const port: EditorPort = {
    context: { kind: "document", id: "d", surface: "desktop" },
    read: () => ({
      text,
      selection: { anchor: 0, head: text.length },
      revision,
    }),
    replaceSelection: (value, expected) => {
      if (expected.revision !== revision) return false;
      text = value;
      revision++;
      return true;
    },
    reveal: () => {},
  };
  await host.runCommand("probe", "upper", port);
  check(text === "ABC", "Worker command edits through the host port");
  results.push("Worker command applies a version-bound edit");

  const frame = document.querySelector("[data-cowboy-editor-plugin-sandbox]");
  check(frame, "sandbox frame exists");
  const started = performance.now();
  let stalled = false;
  const ticker = setInterval(() => {
    if (performance.now() - started > 3000) stalled = true;
  }, 50);
  let timedOut = false;
  try {
    await host.runCommand("probe", "spin", port);
  } catch {
    timedOut = true;
  }
  clearInterval(ticker);
  check(
    timedOut && !stalled,
    "An infinite loop times out without blocking the App thread",
  );
  check(
    !document.querySelector("[data-cowboy-editor-plugin-sandbox]"),
    "Hung sandbox is removed",
  );
  for (
    let i = 0;
    i < 20 && host.getSnapshot().plugins[0]!.status !== "failed";
    i++
  ) {
    await new Promise((r) => setTimeout(r, 50));
  }
  check(
    host.getSnapshot().plugins[0]!.status === "failed",
    "Hung plugin is marked failed",
  );
  results.push(
    `Infinite loop terminated after ${
      Math.round(performance.now() - started)
    }ms; App thread kept running`,
  );
  await host.uninstall("probe");
  check(host.getSnapshot().plugins.length === 0, "uninstalled");
  results.push("Uninstall removes the record");
  return results;
}
