/** List visible Desktop controls that have no keyboard slot, from the real
 * integrated App (the draft-documents fixture held at an open session) in a
 * running Chrome. Writes a screenshot next to the report.
 *
 * Usage: deno run --allow-read --allow-write --allow-run --allow-net=127.0.0.1 \
 *   --allow-env tools/cdp-shortcut-audit.ts http://127.0.0.1:9222 <out-dir>
 *
 * Exits 1 when any control lacks a slot (FOCUS.md "Leader").
 */
import { openFixturePage } from "./cdp-fixture.ts";

const endpoint = Deno.args[0] ?? "";
const output = Deno.args[1];
if (!output?.startsWith("/")) {
  throw new Error("expected an absolute output directory");
}
await Deno.mkdir(output, { recursive: true });
const page = await openFixturePage(
  endpoint,
  "draft-documents",
  () =>
    `<!doctype html><script>
globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
globalThis.__cowboyShortcutAudit = true;
</script><script type="module">
const { runDraftDocumentsBrowserConformance } = await import("/fixture.js");
runDraftDocumentsBrowserConformance().catch((error) => { globalThis.__cowboyShortcutAuditResult = ["error: " + error]; });
</script>`,
);
let missing = 0;
try {
  let result: string[] | null = null;
  for (let attempt = 0; attempt < 600 && !result; attempt++) {
    result = await page.evaluate<string[] | null>(
      "globalThis.__cowboyShortcutAuditResult ?? null",
    );
    if (!result) await new Promise((resolve) => setTimeout(resolve, 300));
  }
  if (!result) throw new Error("audit did not reach the integrated App");
  const { data } = await page.send("Page.captureScreenshot", { format: "png" });
  await Deno.writeFile(
    `${output}/shortcut-audit.png`,
    Uint8Array.from(atob(data), (c) => c.charCodeAt(0)),
  );
  console.log(
    JSON.stringify({ browser: page.browser, missing: result }, null, 2),
  );
  missing = result.length;
} finally {
  await page.close();
}
if (missing > 0) Deno.exit(1);
