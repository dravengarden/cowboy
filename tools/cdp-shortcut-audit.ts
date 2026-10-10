/** List visible Desktop controls that have no keyboard slot, from the real
 * integrated App (the draft-documents fixture held at an open session) in a
 * running Chrome. Writes a screenshot next to the report.
 *
 * Usage: bun tools/cdp-shortcut-audit.ts http://127.0.0.1:9222 <out-dir>
 *
 * Exits 1 when any control lacks a slot (FOCUS.md "Leader").
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { openFixturePage } from "./cdp-fixture.ts";

const endpoint = process.argv.slice(2)[0] ?? "";
const output = process.argv.slice(2)[1];
if (!output?.startsWith("/")) {
  throw new Error("expected an absolute output directory");
}
await mkdir(output, { recursive: true });
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
  await writeFile(
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
  await writeFile(
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
  await writeFile(
    `${output}/move-pick.png`,
    Uint8Array.from(atob(pick.data), (c) => c.charCodeAt(0)),
  );
  await press("Escape", "Escape", 27);
  await waitFor("!document.querySelector('[data-move-pick-banner]')", "Esc leaves Move pick");
  console.log(JSON.stringify({ move_pick: "ok" }));
  // Window motion (Vim off): Ctrl+H/J/K/L walk the regions by geometry.
  const regionNow = "document.activeElement?.closest('[data-desktop-region]')?.dataset.desktopRegion";
  await press("k", "KeyK", 75, mac ? 4 : 1);
  await press("p", "KeyP", 80);
  await waitFor(`${regionNow} === 'prompt.composer'`, "␣P focuses the Prompt");
  const hops: [string, string, number, string][] = [
    ["l", "KeyL", 76, "conversation.transcript"],
    ["h", "KeyH", 72, "prompt.composer"],
    ["h", "KeyH", 72, "sessions.list"],
    ["l", "KeyL", 76, "prompt.composer"],
    ["k", "KeyK", 75, "topbar.controls"],
    ["j", "KeyJ", 74, "prompt.composer"],
  ];
  for (const [key, code, keyCode, region] of hops) {
    await press(key, code, keyCode, 2);
    await waitFor(`${regionNow} === '${region}'`, `Ctrl+${key.toUpperCase()} reaches ${region}`);
  }
  console.log(JSON.stringify({ window_motion: hops.map(([key, , , region]) => `^${key}→${region}`).join(" ") }));
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
      await writeFile(`${output}/title-normal.png`, Uint8Array.from(atob(hint.data), (c) => c.charCodeAt(0)));
    }
    await back();
    await waitFor(inBody, `${name} in the title's Normal returns to the body`);
  }
  console.log(JSON.stringify({ draft_title_return: "j, Enter, Esc" }));
  // With Vim on, the Draft body's Normal moves with Ctrl+H to Sessions.
  await press("Escape", "Escape", 27);
  await press("h", "KeyH", 72, 2);
  await waitFor(`${regionNow} === 'sessions.list'`, "Ctrl+H from the Draft body's Vim Normal reaches Sessions");
  await press("l", "KeyL", 76, 2);
  await waitFor(`${regionNow} === 'prompt.composer'`, "Ctrl+L returns to the Draft body");
  console.log(JSON.stringify({ window_motion_vim: "ok" }));
  // A leader continuation never types. macOS gives keys typed into an
  // editable field to an active CJK input method first, which composes them
  // whatever the page does with the keydown; so while the leader waits the
  // field (or the editor holding the selection under the Vim sink) is not
  // editable and keeps focus, and the continuation runs its command only.
  // CDP cannot drive a real input method, so this checks the lock itself.
  {
    const row = await page.evaluate<{ x: number; y: number }>(`(() => {
      const rect = document.querySelector('[data-desktop-item="integrated-session"]').getBoundingClientRect();
      return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
    })()`);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await page.send("Input.dispatchMouseEvent", { type, x: row.x, y: row.y, button: "left", clickCount: 1 });
    }
    const content = "document.querySelector('[data-desktop-region=\"prompt.composer\"] .cm-content')";
    await waitFor(`!document.querySelector('[data-workspace-document]') && ${content}`, "the Session opens");
    const typed = `${content}.textContent.replace("Message the agent…", "")`;
    const cases: [string, "normal" | "insert", string, string, number, string][] = [
      ["␣C from Normal", "normal", "c", "KeyC", 67, "conversation.transcript"],
      ["⌘K C from Insert", "insert", "c", "KeyC", 67, "conversation.transcript"],
      ["⌘K P from Insert", "insert", "p", "KeyP", 80, "prompt.composer"],
    ];
    for (const [name, mode, key, code, keyCode, region] of cases) {
      await press("k", "KeyK", 75, mac ? 4 : 1);
      await press("p", "KeyP", 80);
      await press("Escape", "Escape", 27);
      await waitFor("document.activeElement?.hasAttribute('data-vim-command-sink')", `${name}: Prompt in Vim Normal`);
      const focused = mode === "insert" ? content : "document.activeElement";
      if (mode === "insert") {
        await press("i", "KeyI", 73);
        await waitFor(`document.activeElement === ${content}`, `${name}: Insert`);
        await press("k", "KeyK", 75, mac ? 4 : 1);
      } else {
        await page.send("Input.dispatchKeyEvent", { type: "keyDown", key: " ", code: "Space", windowsVirtualKeyCode: 32, text: " " });
        await page.send("Input.dispatchKeyEvent", { type: "keyUp", key: " ", code: "Space", windowsVirtualKeyCode: 32 });
      }
      await waitFor(
        `${content}.getAttribute('contenteditable') === 'false' && document.activeElement === ${focused === "document.activeElement" ? "document.activeElement" : content}`,
        `${name}: the editor is locked while the leader waits, focus kept`,
      );
      await page.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode: keyCode, text: key });
      await page.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode });
      await waitFor(
        `document.activeElement?.closest('[data-desktop-region]')?.dataset.desktopRegion === '${region}'`,
        `${name} reaches ${region}`,
      );
      const text = await page.evaluate<string>(typed);
      if (text !== "") throw new Error(`${name} typed "${text}" into the Composer`);
      await waitFor(`${content}.getAttribute('contenteditable') === 'true'`, `${name}: the editor is editable again`);
    }
    console.log(JSON.stringify({ leader_lock: cases.map(([name]) => name).join("; ") }));
  }
  // Rich text renders distinctly, with the product's live-preview CSS (the
  // fixture bundle does not load stylesheets): bold heavier and in full ink,
  // inline code a chip in the body colour, headings larger than the body.
  const content = "document.querySelector('[data-desktop-region=\"prompt.composer\"] .cm-content')";
  {
    const css = await readFile("web/src/mdlive/styles/inline-preview.css", "utf8");
    await page.evaluate(`(() => { const s = document.createElement("style"); s.textContent = ${JSON.stringify(css)}; document.head.append(s); return true; })()`);
    await press("k", "KeyK", 75, mac ? 4 : 1);
    await press("p", "KeyP", 80);
    await press("Escape", "Escape", 27);
    await press("i", "KeyI", 73);
    await waitFor(`document.activeElement === ${content}`, "Insert in the Composer");
    await page.send("Input.insertText", { text: "我有一个问题，**这个用户** normal **bold** *斜体 italic* ~~删除~~ `code 代码` ==高亮== [链接](https://x.y)\n# 标题一 H1\n- 列表\n> 引用 quote\n" });
    await press("Escape", "Escape", 27);
    await new Promise((resolve) => setTimeout(resolve, 400));
    const style = await page.evaluate<{
      body: [string, string, string]; strong: [string, string, string]; code: [string, string];
      link: string; h1: string;
    }>(`(() => {
      const root = ${content};
      const read = (e) => { const c = getComputedStyle(e); return [c.fontWeight, c.color, c.webkitTextStrokeWidth, c.fontSize, c.backgroundColor]; };
      const strong = read(root.querySelector('.cm-atomic-strong'));
      const body = read(root.querySelector('.cm-line'));
      const code = read(root.querySelector('.cm-atomic-inline-code'));
      const link = root.querySelector('.cm-atomic-link, [class*=link]');
      return {
        body: [body[0], body[1], body[3]],
        strong: [strong[0], strong[1], strong[2]],
        code: [code[1], code[4]],
        link: link ? getComputedStyle(link).color : "",
        h1: getComputedStyle(root.querySelector('.cm-atomic-h1')).fontSize,
      };
    })()`);
    const fail = (message: string) => { throw new Error(`${message}: ${JSON.stringify(style)}`); };
    if (Number(style.strong[0]) < 700 || style.strong[1] === style.body[1] || style.strong[2] === "0px") fail("bold is not distinct");
    if (style.code[1] === "rgba(0, 0, 0, 0)" || style.code[0] === style.link) fail("inline code is not a body-coloured chip");
    if (parseFloat(style.h1) <= parseFloat(style.body[2])) fail("H1 is not larger than the body");
    const shot = await page.send("Page.captureScreenshot", { format: "png" });
    await writeFile(`${output}/rich-text.png`, Uint8Array.from(atob(shot.data), (c) => c.charCodeAt(0)));
    console.log(JSON.stringify({ rich_text: style }));
  }
  // Ctrl+H/J/K/L from Vim Insert: no mode change first, nothing deleted.
  {
    await press("i", "KeyI", 73);
    await waitFor(`document.activeElement === ${content}`, "Insert again");
    const before = await page.evaluate<string>(`${content}.textContent`);
    await press("h", "KeyH", 72, 2);
    await waitFor(`${regionNow} === 'sessions.list'`, "Ctrl+H from Vim Insert reaches Sessions");
    if (await page.evaluate<string>(`${content}.textContent`) !== before) {
      throw new Error("Ctrl+H from Insert edited the Composer");
    }
    console.log(JSON.stringify({ window_motion_insert: "ok" }));
  }
  // Vim view motions in the Sessions list: zM / zR fold every folder,
  // Ctrl-D moves the cursor down half a page.
  {
    const collapsed = "document.querySelectorAll('[data-desktop-folder-row][aria-expanded=\"false\"]').length";
    const expanded = "document.querySelectorAll('[data-desktop-folder-row][aria-expanded=\"true\"]').length";
    await press("z", "KeyZ", 90);
    await press("M", "KeyM", 77, 8);
    await waitFor(`${collapsed} > 0 && ${expanded} === 0`, "zM closes every folder");
    await press("z", "KeyZ", 90);
    await press("R", "KeyR", 82, 8);
    await waitFor(`${expanded} > 0 && ${collapsed} === 0`, "zR opens every folder");
    await press("g", "KeyG", 71);
    await press("g", "KeyG", 71);
    const rowIndex = "[...document.querySelectorAll('[data-desktop-region=\"sessions.list\"] [data-desktop-item]')].indexOf(document.activeElement?.closest('[data-desktop-item]'))";
    await waitFor(`${rowIndex} === 0`, "gg reaches the first row");
    await press("d", "KeyD", 68, 2);
    await waitFor(`${rowIndex} > 0`, "Ctrl-D moves the cursor down");
    await press("u", "KeyU", 85, 2);
    await waitFor(`${rowIndex} === 0`, "Ctrl-U moves it back");
    await press("z", "KeyZ", 90);
    await press("z", "KeyZ", 90);
    await waitFor(`${regionNow} === 'sessions.list'`, "zz keeps the cursor in Sessions");
    console.log(JSON.stringify({ list_view_motion: "zM zR gg ^D ^U zz" }));
  }
} finally {
  await page.close();
}
if (missing > 0) process.exit(1);
