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
  ",": ["Comma", 188],
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
const click = async (selector: string) => {
  const point = await page.evaluate<{ x: number; y: number } | null>(`(() => {
    const node = document.querySelector(${JSON.stringify(selector)});
    if (!node) return null;
    const r = node.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
  })()`);
  check(point, `Missing ${selector}`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", { type, button: "left", clickCount: 1, ...point });
  }
  await sleep(150);
};
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

  // Side by side: the old side mirrors, the new side keeps the Vim buffer.
  await keys(" ", "v", "d");
  await until(`document.querySelector("[data-review-diff-layout='split'] [data-desktop-review-split-old]")`, "␣VD shows the split diff");
  const sides = await page.evaluate<{ old: string; next: string }>(`({
    old: document.querySelector("[data-desktop-review-split-old] .cm-content").textContent,
    next: document.querySelector("[data-desktop-review-editor] .cm-content").textContent,
  })`);
  check(sides.old.includes("a") && !sides.old.includes("+b") && sides.next.includes("b") && !sides.next.includes("-a"),
    `the split shows each side without diff markers (${sides.old} | ${sides.next})`);
  // The split has no file header rows, so its first row is hunk 1's header.
  await keys("g", "g");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("1/2")`, "gg starts in hunk 1");
  await keys("]", "c");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("2/2")`, "]c works in the split");
  await keys("[", "c");
  await until(`document.querySelector("[data-review-hunk-position]")?.textContent.includes("1/2")`, "[c returns in the split");
  await key("Enter");
  await until(focusIn("prompt.composer"), "Enter sends from the split");
  const splitRemark = await page.evaluate<string>(`document.querySelector("[data-fixture-prompt]").value`);
  check(/src\/file-\d+\.ts:1\b/.test(splitRemark) && splitRemark.includes("+b"), `a split remark names the new file line: ${splitRemark.slice(0, 120)}`);
  await prefix();
  await keys("v", "v");
  await until(focusIn("review.document"), "back in the Document");
  await shot("9-split");
  await keys(" ", "v", "d");
  await until(`document.querySelector("[data-review-diff-layout='unified']")`, "␣VD returns to the unified diff");
  results.push("Split diff: ␣VD shows both sides aligned, ]c steps hunks, Enter sends the new-side file line, ␣VD returns");

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

  // The Inspector beside the Document: outline, problems, ]d and gO.
  await until(`getComputedStyle(document.querySelector("[data-review-inspector-placement='beside']")).display !== "none"`, "a wide Review shows the Inspector beside the Document");
  await page.evaluate(`document.querySelector("[data-review-tree-path='README.md']").focus()`);
  await key("l");
  await until(`${activeTab} === "README.md"`, "L opens README.md");
  await until(`document.querySelector("[data-review-inspector-section='outline']")?.textContent.includes("Project guide")`, "the Inspector shows the outline");
  await keys(" ", "v", "k");
  await until(`document.querySelector("[data-review-problem='3']")`, "␣VK runs the check and its finding is listed");
  await keys(" ", "v", "v");
  await keys("g", "g");
  await until(`document.querySelector("[data-review-cursor-line='1']")`, "gg reaches line 1");
  await keys("]", "d");
  await until(`document.querySelector("[data-review-cursor-line='3']")`, "]d moves to the finding's line");
  await until(`document.querySelector("[data-desktop-review-editor] .cowboy-review-diagnostic-warning")`, "the language diagnostic is underlined");
  await until(`document.querySelector("[data-review-inspector-section='problems']")?.textContent.includes("Fixture diagnostic")`, "the diagnostic joins Problems");
  await keys("g", "g");
  await key("K");
  await until(`document.querySelector("[data-review-hover]")?.textContent.includes("Project guide heading")`, "K shows the hover");
  await shot("10-hover");
  await key("Escape");
  await until(`!document.querySelector("[data-review-hover]")`, "Escape closes the hover");
  await until(focusIn("review.document"), "focus returns to the Document");
  await keys("g", "d");
  await until(`${activeTab} === "src/app.ts"`, "gd opens the definition");
  await until(`${focusIn("review.document")} && document.querySelector("[data-desktop-review-editor] .cm-content")?.textContent.includes("Project guide")`, "the definition's text is focused");
  await keys("g", "T");
  await until(`${activeTab} === "README.md"`, "gT returns to the file");
  results.push("Language: the diagnostic is underlined and listed, K shows the hover, Escape returns, gd opens the definition");
  await keys("g", "O");
  await until(`document.activeElement?.closest("[data-review-inspector-section='outline']")`, "gO focuses the outline");
  await key("Enter");
  await until(`document.querySelector("[data-review-cursor-line='1']")`, "Enter on a symbol moves the cursor to it");
  await shot("8-inspector");
  results.push("Inspector: outline and problems beside the Document; ␣VK runs the check, ]d reaches its finding, gO focuses the outline and Enter jumps");

  // ␣VR on a focused change asks a fork for a managed review; its findings
  // come back as a Navigator badge and as Problems on their lines.
  await keys(" ", "v", "c");
  await until(focusIn("review.navigator"), "␣VC focuses the Navigator");
  await page.evaluate(`document.querySelector("[data-review-change='src/app.ts']").focus()`);
  await keys(" ", "v", "r");
  await until(`globalThis.__reviewFixture.submits.some((data) => data.includes('"session_id":"explain-fork"') && data.includes("review.start") && data.includes('src/app.ts'))`, "␣VR sends the delegation contract to the fork");
  await until(`document.querySelector("[data-review-change='src/app.ts'] [data-review-finding-badge='1']")`, "the review's finding badges its file");
  await page.evaluate(`document.querySelector("[data-review-change='src/app.ts']").focus()`);
  await key("l");
  await until(`${activeTab} === "src/app.ts"`, "L opens the reviewed file");
  await until(`document.querySelector("[data-review-inspector-section='problems']")?.textContent.includes("Fixture AI finding")`, "the finding joins Problems");
  // An unverified (legacy) diff lists findings without placing them; the
  // file itself places a current one on its line.
  await keys(" ", "v", "f");
  await until(`document.querySelector("[data-review-tree-path='src']")`, "the tree loads");
  await page.evaluate(`document.querySelector("[data-review-tree-path='src']").focus()`);
  await key("l");
  await until(`document.querySelector("[data-review-tree-path='src/app.ts']")`, "L expands src");
  await page.evaluate(`document.querySelector("[data-review-tree-path='src/app.ts']").focus()`);
  await key("l");
  await until(`[...document.querySelectorAll("[data-review-problem]")].some((row) => row.textContent.includes("Fixture AI finding") && !row.classList.contains("Mui-disabled"))`, "a current finding is placed on its line in the file");
  await shot("11-ai-review");
  results.push("␣VR delegates a managed review of the focused change to a fork; its finding badges the file, joins Problems and is placed on its line");

  await keys(" ", "v", "h");
  await until(`document.querySelector("[data-review-navigator-tab='history'][aria-selected='true']")`, "␣VH shows History");
  await until(`document.querySelector("[data-review-commit]")`, "commits load");
  await page.evaluate(`document.querySelector("[data-review-commit]").focus()`);
  await key("l");
  await until(`document.querySelector("[data-review-commit-file='src/app.ts']")`, "L opens the commit's files");
  await key("j");
  await key("Enter");
  await until(`document.querySelector("[data-review-file-tab-kind='commit'][aria-selected='true']")`, "Enter opens the commit file");
  await until(`document.querySelector("[data-desktop-review-editor]")?.textContent.includes("committed")`, "the commit diff is shown");
  await keys(" ", "v", "h");
  await page.evaluate(`document.querySelector("[data-review-commit-file]").focus()`);
  await key("h");
  await until(`document.activeElement?.dataset.reviewCommit`, "H returns to the commits");
  results.push("History: ␣VH, L opens a commit's files, Enter opens its diff as a commit tab, H returns");

  await keys(" ", "v", "g");
  await until(`document.querySelector("[data-review-navigator-tab='pulls'][aria-selected='true']")`, "␣VG shows Pull request");
  await until(`document.querySelector("[data-review-pull='dravengarden/cowboy#42']")`, "the picker lists the account's pull requests");
  await page.evaluate(`document.querySelector("[data-review-pull='dravengarden/cowboy#42']").focus()`);
  await key("Enter");
  await until(`document.querySelector("[data-review-pull-file='web/src/pr-one.ts']")`, "Enter binds the PR and lists its files");
  await page.evaluate(`document.querySelector("[data-review-pull-file='web/src/pr-one.ts']").focus()`);
  await key("l");
  await until(`document.querySelector("[data-review-file-tab-kind='pr'][aria-selected='true']")`, "L opens the PR file as a PR tab");
  await until(`document.querySelector("[data-desktop-review-editor]")?.textContent.includes("pull one")`, "the PR patch is shown");
  await keys("]", "f");
  await until(`document.querySelector("[data-desktop-review-editor]")?.textContent.includes("pull two")`, "]f moves to the PR's next file");
  await keys(" ", "v", "m");
  await until(`document.querySelector("[data-review-pull-facts]")?.textContent.includes("1/2 viewed")`, "␣VM marks the PR file viewed");
  await shot("7-pull-request");
  results.push("Pull request: ␣VG lists the account's PRs; Enter binds #42, L opens a file, ]f the next, ␣VM marks it viewed");

  await keys(" ", "v", "w");
  await until(`document.querySelector("[data-review-worktree]")`, "␣VW lists the worktrees");
  await keys(" ", "v", "x");
  await until(`document.querySelector("[data-review-context-option='default']")`, "␣VX opens the context menu");
  const option = await page.evaluate<string>(`[...document.querySelectorAll("[data-review-context-option]")].map((n) => n.dataset.reviewContextOption).find((v) => v !== "default") ?? ""`);
  check(option, "the context menu offers another context");
  await shot("5-context");
  const before = await text("[data-review-context]");
  await click(`[data-review-context-option="${option}"]`);
  await until(`document.querySelector("[data-review-context]")?.textContent.includes("chosen here")`, "choosing a context marks it chosen");
  check((await text("[data-review-context]")) !== before, "the context control names the chosen context");
  await until(`document.querySelector("[data-review-progress]")?.textContent.includes("reviewed")`, "the chosen context's changes load");
  await until(`!document.querySelector("[role=menu]")`, "the menu closes");
  await until(focusIn("review.navigator"), "focus lands in the new context's Navigator");
  await keys(" ", "v", "x");
  await until(`document.querySelector("[data-review-context-option='default']")`, "␣VX reopens the menu");
  await click(`[data-review-context-option="default"]`);
  await until(`!document.querySelector("[data-review-context]")?.textContent.includes("chosen here")`, "This session restores the default");
  await until(`!document.querySelector("[role=menu]")`, "the menu closes again");
  await until(focusIn("review.navigator"), "focus returns to the Navigator");
  results.push(`Worktrees list; ␣VX switches Review to ${option} and back to This session`);

  await keys(" ", "v", ",");
  await until(`[...document.querySelectorAll("[role=dialog]")].some((d) => d.textContent.includes("Review settings"))`, "␣V, opens Review settings");
  await shot("6-settings");
  await key("Escape");
  await until(`![...document.querySelectorAll("[role=dialog]")].some((d) => d.textContent.includes("Review settings"))`, "Escape closes the settings");
  results.push("␣V, opens Review settings and Escape closes them");

  const navigatorWidth = () => page.evaluate<number>(`document.querySelector("[data-review-navigator]").getBoundingClientRect().width`);
  const startWidth = await navigatorWidth();
  const edge = await page.evaluate<{ x: number; y: number }>(`(() => { const r = document.querySelector("[data-desktop-splitter='review-navigator']").getBoundingClientRect(); return { x: r.x, y: r.y + r.height / 2 }; })()`);
  await page.send("Input.dispatchMouseEvent", { type: "mousePressed", button: "left", clickCount: 1, ...edge });
  await page.send("Input.dispatchMouseEvent", { type: "mouseMoved", button: "left", x: edge.x + 80, y: edge.y });
  await page.send("Input.dispatchMouseEvent", { type: "mouseReleased", button: "left", clickCount: 1, x: edge.x + 80, y: edge.y });
  await sleep(150);
  const widened = await navigatorWidth();
  check(widened >= startWidth + 60, `dragging the Navigator edge widens it (${String(startWidth)} → ${String(widened)})`);
  // A pointer on a bar selects it for Resize mode, as the work-pane bars do.
  check(
    await page.evaluate<boolean>(`document.querySelector("[data-desktop-splitter='review-navigator']")?.dataset.desktopSplitterSelected === "true"`),
    "the dragged edge is selected in Resize mode",
  );
  await key("Escape");
  await until(`!document.querySelector("[data-desktop-splitter-selected='true']")`, "Escape leaves Resize mode");
  results.push(`The Navigator edge drags from ${String(Math.round(startWidth))} to ${String(Math.round(widened))} px and joins Resize mode`);

  await viewport(1180, 900);
  await until(`document.querySelector("[data-review-compact='true']")`, "a narrow Review is compact");
  check(
    await page.evaluate<boolean>(`getComputedStyle(document.querySelector("[data-desktop-region='review.inspector']")).display === "none"`),
    "a narrow Review folds the Inspector",
  );
  await keys(" ", "v", "]");
  await until(`getComputedStyle(document.querySelector("[data-review-inspector-placement='inset']")).display !== "none"`, "␣V] opens the Inspector inset");
  await keys(" ", "v", "]");
  await until(`getComputedStyle(document.querySelector("[data-desktop-region='review.inspector']")).display === "none"`, "␣V] folds it again");
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
