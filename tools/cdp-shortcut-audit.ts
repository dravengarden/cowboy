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
  // Recent (`␣O`) with trusted keys in the same App, after the Draft visits
  // the fixture made: it lists them, newest first, and Esc closes it.
  const mac = (await page.evaluate<string>("navigator.platform"))
    .toLowerCase().includes("mac");
  const press = async (key: string, code: string, keyCode: number, modifiers = 0) => {
    const base = {
      key,
      code,
      windowsVirtualKeyCode: keyCode,
      nativeVirtualKeyCode: keyCode,
      modifiers,
    };
    await page.send("Input.dispatchKeyEvent", { type: "rawKeyDown", ...base });
    await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
    await new Promise((resolve) => setTimeout(resolve, 80));
  };
  await press("k", "KeyK", 75, mac ? 4 : 1);
  await press("o", "KeyO", 79);
  let rows = 0;
  for (let attempt = 0; attempt < 50 && rows === 0; attempt++) {
    rows = await page.evaluate<number>(
      "document.querySelectorAll('[data-desktop-recent-index]').length",
    );
    if (rows === 0) await new Promise((resolve) => setTimeout(resolve, 100));
  }
  if (rows === 0) throw new Error("␣O did not open Recent with its visits");
  await new Promise((resolve) => setTimeout(resolve, 300));
  const recent = await page.send("Page.captureScreenshot", { format: "png" });
  await Deno.writeFile(
    `${output}/recent.png`,
    Uint8Array.from(atob(recent.data), (c) => c.charCodeAt(0)),
  );
  await press("Escape", "Escape", 27);
  console.log(JSON.stringify({ recent_rows: rows }));
  // The Command Palette is a launcher: one Esc from its search closes it.
  const waitFor = async (expression: string, message: string) => {
    for (let attempt = 0; attempt < 50; attempt++) {
      if (await page.evaluate<boolean>(`Boolean(${expression})`)) return;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
    throw new Error(message);
  };
  await press("k", "KeyK", 75, mac ? 4 : 1);
  await press("k", "KeyK", 75);
  const palette = "document.querySelector('input[aria-label=\"Search commands\"]')";
  await waitFor(`${palette} && document.activeElement === ${palette}`, "palette opens in its search");
  await press("Escape", "Escape", 27);
  await waitFor(`!${palette}`, "One Esc closes the palette");
  console.log(JSON.stringify({ palette_grammar: "ok" }));
  // Move pick (`␣SM`) from anywhere: the page darkens, folders get letters,
  // Esc leaves.
  await press("k", "KeyK", 75, mac ? 4 : 1);
  await press("s", "KeyS", 83);
  await press("m", "KeyM", 77);
  await waitFor(
    "document.querySelector('[data-move-pick-banner]') && document.querySelector('[data-desktop-move-spotlight]') && document.querySelector('[data-move-pick-label]')",
    "␣SM starts Move pick",
  );
  await new Promise((resolve) => setTimeout(resolve, 250));
  const pick = await page.send("Page.captureScreenshot", { format: "png" });
  await Deno.writeFile(
    `${output}/move-pick.png`,
    Uint8Array.from(atob(pick.data), (c) => c.charCodeAt(0)),
  );
  await press("Escape", "Escape", 27);
  await waitFor("!document.querySelector('[data-move-pick-banner]')", "Esc leaves Move pick");
  console.log(JSON.stringify({ move_pick: "ok" }));
  // The Draft title in the real App (Vim on): `␣T` puts a Normal cursor on
  // it, and each documented way back reaches the body.
  await page.evaluate(`(() => {
    localStorage.setItem("cowboy:vim", "1");
    dispatchEvent(new StorageEvent("storage", { key: "cowboy:vim", newValue: "1", storageArea: localStorage }));
    return true;
  })()`);
  const box = await page.evaluate<{ x: number; y: number }>(`(() => {
    const row = document.querySelector('[data-desktop-item^="draft:"]');
    const rect = row.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  })()`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", { type, x: box.x, y: box.y, button: "left", clickCount: 1 });
  }
  const titleInput = "document.querySelector(\"input[aria-label='Draft title']\")";
  await waitFor(`${titleInput} && document.querySelector('[data-workspace-document] .cm-content')`, "Draft opens");
  const inBody = "Boolean(document.activeElement?.closest('[data-workspace-document] .cm-editor'))";
  const ways: [string, () => Promise<void>][] = [
    ["j", () => press("j", "KeyJ", 74)],
    ["Enter", () => press("Enter", "Enter", 13)],
    ["Esc", () => press("Escape", "Escape", 27)],
  ];
  for (const [name, back] of ways) {
    await press("k", "KeyK", 75, mac ? 4 : 1);
    await press("t", "KeyT", 84);
    await waitFor(
      `document.activeElement === ${titleInput} && ${titleInput}.dataset.vimInputMode === 'normal'`,
      `␣T puts a Normal cursor on the title (before ${name})`,
    );
    await waitFor("document.querySelector(\"[data-draft-title-hint='normal']\")", "the title names its way back");
    if (name === "j") {
      const hint = await page.send("Page.captureScreenshot", { format: "png" });
      await Deno.writeFile(`${output}/title-normal.png`, Uint8Array.from(atob(hint.data), (c) => c.charCodeAt(0)));
    }
    await back();
    await waitFor(inBody, `${name} in the title's Normal returns to the body`);
  }
  console.log(JSON.stringify({ draft_title_return: "j, Enter, Esc" }));
} finally {
  await page.close();
}
if (missing > 0) Deno.exit(1);
