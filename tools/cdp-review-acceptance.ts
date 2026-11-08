/** Trusted-input acceptance for the Desktop Review workbench
 * (docs/desktop-code-review.md D1) in an anonymous Chrome context over the
 * review-context fixture: no product endpoints, credentials or real code.
 *
 * Usage: bun tools/cdp-review-acceptance.ts <endpoint> <out-dir> [light|dark]
 */
import { mkdir, writeFile } from "node:fs/promises";
import { openFixturePage } from "./cdp-fixture.ts";

const [endpoint = "http://127.0.0.1:9222", output = "/tmp/cowboy-review-acceptance", theme = "light"] =
  process.argv.slice(2);
await mkdir(output, { recursive: true });
const page = await openFixturePage(
  endpoint,
  "desktop-review",
  () =>
    `<!doctype html><script>localStorage.setItem("cowboy:theme-system-default-v1","1");localStorage.setItem("cowboy-theme-mode",${JSON.stringify(theme)});
globalThis.CowboyDeviceProof={proof:async()=>"fixture",resetChallenge(){},install(){}};
addEventListener("error", (e) => (globalThis.__errors ??= []).push(String(e.message)));</script>
<script type="module">const m=await import("/fixture.js");m.mountDesktopReviewWorkbench();</script>`,
);
const results: string[] = [];
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
const until = async (expression: string, message: string, ms = 8000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    if (await page.evaluate<boolean>(`Boolean(${expression})`)) return;
    await sleep(60);
  }
  throw new Error(`Timed out: ${message}`);
};
const CODES: Record<string, [string, number]> = {
  " ": ["Space", 32],
  Escape: ["Escape", 27],
  Enter: ["Enter", 13],
  "]": ["BracketRight", 221],
  "[": ["BracketLeft", 219],
  "/": ["Slash", 191],
};
const key = async (value: string, modifiers = 0) => {
  const upper = value.length === 1 && value !== value.toLowerCase();
  const [code, keyCode] = CODES[value] ?? [`Key${value.toUpperCase()}`, value.toUpperCase().charCodeAt(0)];
  const shift = upper ? 8 : 0;
  const text = modifiers === 0 && value.length === 1 ? value : undefined;
  const base = { key: value, code, windowsVirtualKeyCode: keyCode, nativeVirtualKeyCode: keyCode, modifiers: modifiers | shift };
  await page.send("Input.dispatchKeyEvent", { type: text ? "keyDown" : "rawKeyDown", ...base, ...(text ? { text } : {}) });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  await sleep(90);
};
const keys = async (...values: string[]) => {
  for (const value of values) await key(value);
};
const shot = async (name: string) => {
  const { data } = await page.send("Page.captureScreenshot", { format: "png" });
  await writeFile(`${output}/${theme}-${name}.png`, Uint8Array.from(atob(data), (c) => c.charCodeAt(0)));
};
const text = (selector: string) =>
  page.evaluate<string>(`document.querySelector(${JSON.stringify(selector)})?.textContent ?? ""`);
const focusIn = (region: string) => `document.activeElement?.closest("[data-desktop-region='${region}']")`;
const activeTab = `document.querySelector("[data-review-file-tab][aria-selected='true']")?.dataset.reviewFileTab`;
const mac = (await page.evaluate<string>("navigator.platform")).toLowerCase().includes("mac");
// The leader from a text field: Cmd+K on macOS, Alt+K elsewhere.
const prefix = () => key("k", mac ? 4 : 1);
const viewport = (width: number, height: number) =>
  page.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: false });

try {
  await viewport(1680, 960);
  await until(`document.querySelector("[data-desktop-work-view='review']")`, "work-pane tabs");
  // A reveal before Review was ever opened shows it with the named file.
  await sleep(300);
  await page.evaluate(`globalThis.__reviewFixture.sync("mobile-review:worktree", {mode:"files",tabs:[{path:"README.md",pinned:false}],active:"README.md",progress:{},positions:{},open_revision:1,reveal_revision:1}, 1000)`);
  await until(`document.querySelector("[data-desktop-review-workbench]")`, "a reveal shows the workbench");
  await until(`${activeTab} === "README.md"`, "the reveal opens the named file before Review had mounted");
  results.push("An agent's reveal before Review was opened shows Review with the named file");
  const back = await page.evaluate<{ x: number; y: number }>(`(() => {
    const r = document.querySelector("[data-desktop-work-view='conversation']").getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  })()`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", { type, button: "left", clickCount: 1, ...back });
  }
  await until(`!document.querySelector("[data-desktop-review]:not([hidden])")`, "back to the Conversation");
  await page.evaluate(`document.querySelector("[data-fixture-transcript]").closest("[data-desktop-region]").focus()`);
  await keys(" ", "v");
  await until(`document.querySelector("[data-desktop-review]:not([hidden]) [data-desktop-review-workbench]")`, "␣V shows the workbench");
  await until(`document.querySelector("[data-review-progress]")?.textContent.includes("reviewed")`, "changes load");
  check(
    !(await page.evaluate<boolean>(`Boolean(document.querySelector("[data-review-compact]"))`)),
    "a wide Review keeps the Navigator beside the Document",
  );
  await shot("1-workbench");
  results.push("␣V shows the workbench: Navigator beside the Document, changes and progress loaded");

  await keys(" ", "v", "c");
  await until(focusIn("review.navigator"), "␣VC focuses the Navigator");
  await keys("j", "j");
  const row = await page.evaluate<string>(`document.activeElement?.dataset.reviewChange ?? ""`);
  check(row, "J moves the list cursor onto a change row");
  await key("l");
  await until(`${activeTab} === ${JSON.stringify(row)}`, "L opens the focused change");
  await until(focusIn("review.document"), "opening moves focus into the Document");
  await until(`document.querySelector("[data-review-file-tab-preview='true']")`, "browsing opens a preview tab");
  results.push(`Navigator: ␣VC, J J, L opens ${row} as a preview tab and focuses the Document`);

  await until(`document.querySelector("[data-review-hunk-position]")`, "hunk position");
  await keys("]", "c");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("1/2")`, "]c reaches hunk 1");
  await keys("]", "c");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("2/2")`, "]c reaches hunk 2");
  await keys("[", "c");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("1/2")`, "[c returns");
  results.push("Document: ]c ]c [c step the hunks (1/2 → 2/2 → 1/2)");

  await keys(" ", "v", "p");
  await until(`!document.querySelector("[data-review-file-tab-preview='true']")`, "␣VP keeps the tab");
  await keys("]", "f");
  await until(`${activeTab} !== ${JSON.stringify(row)}`, "]f opens the next change");
  const second = await page.evaluate<string>(activeTab);
  check(
    (await page.evaluate<number>(`document.querySelectorAll("[data-review-file-tab]").length`)) === 2,
    "the kept tab stays beside the new preview",
  );
  await keys("g", "T");
  await until(`${activeTab} === ${JSON.stringify(row)}`, "gT returns to the kept tab");
  await keys("g", "t");
  await until(`${activeTab} === ${JSON.stringify(second)}`, "gt moves forward");
  await shot("2-document");
  results.push(`Tabs: ␣VP keeps, ]f opens ${second}, gT/gt step between the two`);

  await keys(" ", "v", "m");
  await until(`document.querySelector("[data-review-mark]")?.getAttribute("aria-pressed") === "true"`, "␣VM marks reviewed");
  await until(`document.querySelector("[data-review-progress]")?.textContent.startsWith("1 /")`, "progress counts it");
  await keys("[", "f");
  await keys("]", "u");
  await until(`${activeTab} !== ${JSON.stringify(second)}`, "]u skips the reviewed file");
  results.push("␣VM marks the file reviewed; ]u skips it while ]f would not");

  await keys("V", "j", "Enter");
  await until(focusIn("prompt.composer"), "Enter hands focus to the Prompt");
  const remark = await page.evaluate<string>(`document.querySelector("[data-fixture-prompt]").value`);
  check(remark.includes("```diff"), `the Prompt received the selected lines: ${remark.slice(0, 80)}`);
  check(
    await page.evaluate<boolean>(`Boolean(document.querySelector("[data-desktop-review]:not([hidden])"))`),
    "Review stays shown beside the Prompt",
  );
  results.push("V j Enter appends the selection with its location to the Prompt and focuses it; Review stays");

  await prefix();
  await keys("v", "v");
  await until(focusIn("review.document"), "Alt/Cmd+K V V returns to the Document from the Prompt");
  await keys("i");
  await sleep(150);
  check(
    await page.evaluate<boolean>(`!document.querySelector("[data-desktop-review-editor] .cm-content")?.contains(document.activeElement) || document.activeElement?.matches("[data-vim-command-sink]")`),
    "Insert is refused in the read-only Document",
  );
  for (let i = 0; i < 3; i++) await key("Escape");
  check(
    await page.evaluate<boolean>(`Boolean(document.querySelector("[data-desktop-review]:not([hidden])"))`),
    "Escape never leaves Review",
  );
  results.push("The Document refuses Insert, and Escape never leaves Review");

  await keys(" ", "v", "f");
  await until(`document.querySelector("[data-review-navigator-tab='files'][aria-selected='true']")`, "␣VF shows Files");
  await until(`document.querySelector("[data-review-tree-path='src']")`, "the tree loads");
  await page.evaluate(`document.querySelector("[data-review-tree-path='src']").focus()`);
  await key("l");
  await until(`document.querySelector("[data-review-tree-path='src/app.ts']")`, "L expands a folder");
  await key("j");
  await key("h");
  await until(`document.activeElement?.dataset.reviewTreePath === "src"`, "H moves to the parent folder");
  await key("h");
  await until(`!document.querySelector("[data-review-tree-path='src/app.ts']")`, "H collapses it");
  await shot("3-files");
  results.push("Files: ␣VF, L expands, H goes to the parent and collapses");

  await viewport(1180, 900);
  await until(`document.querySelector("[data-review-compact='true']")`, "a narrow Review is compact");
  await keys(" ", "v", "c");
  await until(`document.querySelector("[data-review-navigator-overlay]")`, "␣VC opens the inset Navigator");
  await shot("4-compact");
  await key("Escape");
  await until(`!document.querySelector("[data-review-navigator-overlay]")`, "Escape closes the inset");
  results.push("Compact width: the Navigator folds; ␣VC opens it inset and Escape closes it");

  // ␣C belongs to the App's command host (tools/cdp-surfaces-flow.ts covers
  // it); this fixture returns through the view tab.
  const tab = await page.evaluate<{ x: number; y: number }>(`(() => {
    const r = document.querySelector("[data-desktop-work-view='conversation']").getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  })()`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", { type, button: "left", clickCount: 1, ...tab });
  }
  await until(`!document.querySelector("[data-desktop-review]:not([hidden])")`, "the Conversation tab returns");
  check(
    await page.evaluate<boolean>(`Boolean(document.querySelector("[data-desktop-review][hidden] [data-review-file-tab]"))`),
    "Review keeps its tabs mounted behind the Conversation",
  );
  results.push("The Conversation tab returns; Review keeps its open files behind it");

  console.log(JSON.stringify({ ok: true, browser: page.browser, theme, screenshots: output, tests: results }, null, 2));
} catch (error) {
  await shot("failure").catch(() => {});
  const errors = await page.evaluate<string[]>("globalThis.__errors ?? []").catch(() => []);
  console.log(JSON.stringify({ ok: false, error: String(error), page_errors: errors, passed: results }, null, 2));
  process.exitCode = 1;
} finally {
  await page.close();
}
