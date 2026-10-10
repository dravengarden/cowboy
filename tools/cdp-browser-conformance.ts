/** Run an existing browser conformance fixture in an already running Chrome
 * over a loopback DevTools endpoint; isolation is described in cdp-fixture.ts.
 *
 * Usage: deno run --allow-read --allow-write --allow-run --allow-net=127.0.0.1 \
 *   --allow-env tools/cdp-browser-conformance.ts http://127.0.0.1:9223 <suite> [light|dark]
 */
import { openFixturePage } from "./cdp-fixture.ts";

const endpoint = Deno.args[0] ?? "";
const suite = Deno.args[1] ?? "desktop-composer";
const themeMode = Deno.args[2] ?? "light";
const ENTRIES: Readonly<Record<string, [string, number]>> = {
  "desktop-composer": ["runDesktopComposerBrowserConformance", 12],
  "draft-documents": ["runDraftDocumentsBrowserConformance", 12],
  "editor-plugin-sandbox": ["runEditorPluginSandboxBrowserConformance", 4],
  "managed-calls": ["runManagedCallsBrowserConformance", 11],
  "session-fold": ["runSessionFoldBrowserConformance", 6],
  "session-move": ["runSessionMoveBrowserConformance", 4],
  "sheet-keyboard": ["runCoverKeyboardBrowserConformance", 3],
  "tool-inspector": ["runToolInspectorBrowserConformance", 3],
  "workspace-picker": ["runWorkspacePickerBrowserConformance", 10],
  "project-placement": ["runProjectPlacementBrowserConformance", 11],
};
const selected = ENTRIES[suite];
if (!selected) throw new Error("unknown suite");
if (themeMode !== "light" && themeMode !== "dark") {
  throw new Error("unknown theme mode");
}
const [entry, expectedTests] = selected;

// The Draft suite installs the example editor plugin packed by the same tool
// authors use (see idb-browser-conformance.ts).
const files: Record<string, { type: string; body: Uint8Array }> = {};
if (suite === "draft-documents") {
  const packed = await Deno.makeTempFile({ suffix: ".cowboy-plugin" });
  try {
    const pack = await new Deno.Command(Deno.execPath(), {
      args: [
        "run",
        "--allow-read",
        `--allow-write=${packed}`,
        "tools/editor-plugin-pack.ts",
        "examples/editor-plugins/text-tools",
        packed,
      ],
      stdout: "null",
      stderr: "inherit",
    }).output();
    if (!pack.success) throw new Error("editor plugin pack failed");
    files["/editor-plugin.cowboy-plugin"] = {
      type: "application/json",
      body: await Deno.readFile(packed),
    };
  } finally {
    await Deno.remove(packed);
  }
}

const page = await openFixturePage(
  endpoint,
  suite,
  (token) =>
    `<!doctype html><script>
localStorage.setItem("cowboy:theme-system-default-v1", "1");
localStorage.setItem("cowboy-theme-mode", ${JSON.stringify(themeMode)});
globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
</script><script type="module">
let result;
try { const { ${entry} } = await import("/fixture.js"); result = { ok: true, tests: await ${entry}() }; }
catch (error) { result = { ok: false, error: String(error) }; }
await fetch("/report/${token}", { method: "POST", body: JSON.stringify(result) });
</script>`,
  files,
);
try {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const result = await Promise.race([
    page.report,
    new Promise((_, reject) => {
      timer = setTimeout(
        () => reject(new Error("browser conformance timed out")),
        suite === "draft-documents" ? 180_000 : 90_000,
      );
    }),
  ]).finally(() => clearTimeout(timer)) as {
    ok?: boolean;
    tests?: unknown[];
  } | null;
  if (
    result?.ok !== true || !Array.isArray(result.tests) ||
    result.tests.length !== expectedTests
  ) {
    throw new Error(`browser conformance failed: ${JSON.stringify(result)}`);
  }
  console.log(JSON.stringify(
    {
      ok: true,
      suite,
      theme_mode: themeMode,
      browser: page.browser,
      endpoint,
      fixture_sha256: page.digest,
      isolation:
        "disposable browser context / intercepted fictitious origin / no credentials",
      tests: result.tests,
    },
    null,
    2,
  ));
} finally {
  await page.close();
}
