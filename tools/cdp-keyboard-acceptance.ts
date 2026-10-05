/** Trusted-input Desktop keyboard acceptance in a running Chrome.
 *
 * Unlike the conformance fixtures, every key here is a browser-level
 * `Input.dispatchKeyEvent` (isTrusted), so the page sees what a physical
 * keyboard produces, including the platform modifier. Screenshots of the
 * leader, label and dialog layers are written to the output directory.
 * IME composition uses `Input.imeSetComposition`; it exercises the page's
 * composition path but is not a substitute for a real OS input method.
 *
 * Usage: deno run --allow-read --allow-write --allow-run --allow-net=127.0.0.1 \
 *   --allow-env tools/cdp-keyboard-acceptance.ts http://127.0.0.1:9223 <out-dir>
 */
import { type CdpParams, openFixturePage } from "./cdp-fixture.ts";

const endpoint = Deno.args[0] ?? "";
const output = Deno.args[1];
if (!output?.startsWith("/")) {
  throw new Error("expected an absolute output directory");
}
await Deno.mkdir(output, { recursive: true });

const page = await openFixturePage(
  endpoint,
  "keyboard-acceptance",
  () =>
    `<!doctype html><script type="module">
addEventListener("error", (e) => (globalThis.__errors ??= []).push(String(e.message)));
addEventListener("unhandledrejection", (e) => (globalThis.__errors ??= []).push(String(e.reason?.stack ?? e.reason)));
const { mountDesktopKeyboardAcceptance } = await import("/fixture.js");
mountDesktopKeyboardAcceptance();
</script>`,
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
const mac = (await page.evaluate<string>("navigator.platform")).toLowerCase()
  .includes("mac");
const MOD = mac ? 4 : 1; // CDP modifiers: Alt=1, Ctrl=2, Meta=4, Shift=8
const KEYS: Readonly<Record<string, [string, number]>> = {
  " ": ["Space", 32],
  "Escape": ["Escape", 27],
  "Enter": ["Enter", 13],
  "'": ["Quote", 222],
};
const key = async (value: string, modifiers = 0, extra: CdpParams = {}) => {
  const [code, keyCode] = KEYS[value] ??
    [
      /^[a-z]$/i.test(value) ? `Key${value.toUpperCase()}` : `Digit${value}`,
      value.toUpperCase().charCodeAt(0),
    ];
  const text = value.length === 1 && modifiers === 0 ? value : undefined;
  const base = {
    key: value,
    code,
    windowsVirtualKeyCode: keyCode,
    nativeVirtualKeyCode: keyCode,
    modifiers,
    ...extra,
  };
  await page.send("Input.dispatchKeyEvent", {
    type: text ? "keyDown" : "rawKeyDown",
    ...base,
    ...(text ? { text } : {}),
  });
  await page.send("Input.dispatchKeyEvent", { type: "keyUp", ...base });
  await sleep(80);
};
const prefix = () => key("k", MOD);
const click = async (selector: string) => {
  const box = await page.evaluate<{ x: number; y: number } | null>(`(() => {
    const element = document.querySelector(${JSON.stringify(selector)});
    if (!element) return null;
    const rect = element.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  })()`);
  check(box, `Missing ${selector}`);
  for (const type of ["mousePressed", "mouseReleased"]) {
    await page.send("Input.dispatchMouseEvent", {
      type,
      x: box.x,
      y: box.y,
      button: "left",
      clickCount: 1,
    });
  }
  await sleep(120);
};
const shot = async (name: string) => {
  const { data } = await page.send("Page.captureScreenshot", { format: "png" });
  await Deno.writeFile(
    `${output}/${name}.png`,
    Uint8Array.from(atob(data), (c) => c.charCodeAt(0)),
  );
};
const mode = () => page.evaluate<string>("globalThis.__workspace?.mode");
const editorText = () =>
  page.evaluate<string>(
    "globalThis.__acceptance.editor.current?.getValue() ?? ''",
  );

try {
  await until(
    "globalThis.__workspace && document.querySelector('.cm-content')",
    "surface mounts",
    20_000,
  );
  await sleep(600);

  // 1. Insert mode owns Space: trusted keys type into the shared editor.
  await click(".cm-content");
  for (const value of ["i", "h", "i", " ", "o", "k"]) await key(value);
  check(
    (await editorText()).includes("hi ok"),
    `Space types in Insert (got ${JSON.stringify(await editorText())})`,
  );
  check(await mode() === "normal", "Typing Space never arms the leader");
  results.push("Insert: trusted Space types a space and never arms the leader");

  // 2. Vim Normal: Space arms the leader, which-key appears, slots light.
  await key("Escape");
  const normalState = await page.evaluate<string>(
    `JSON.stringify({ region: globalThis.__workspace.focusedRegion, active: document.activeElement?.outerHTML.slice(0, 120), sinkRegion: document.activeElement?.closest("[data-desktop-region]")?.dataset.desktopFocused })`,
  );
  await key(" ");
  check(
    await mode() === "command",
    `Space in Vim Normal arms the leader ${normalState}`,
  );
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"root\"]')",
    "which-key panel",
  );
  check(
    await page.evaluate<boolean>(
      'Boolean(document.querySelector(\'[data-composer-action="attach"] [data-shortcut-state="active"]\'))',
    ),
    "Armed leader lights the ␣A slot",
  );
  await shot("1-leader-which-key");
  await key("Escape");
  check(await mode() === "normal", "Esc closes the leader");
  check(!(await editorText()).includes("  "), "The leader Space typed nothing");
  results.push(
    "Vim Normal: trusted Space opens which-key, lights ␣ slots, Esc closes, no stray space",
  );

  // 2b. A pending Vim command keeps Space as its argument (f<Space>).
  await key("0");
  await key("f");
  await key(" ");
  check(
    await mode() === "normal",
    "f<Space> stays a Vim motion, not the leader",
  );
  results.push("Vim f<Space>: a pending Vim command keeps Space");

  // 3. IME composition owns Space and Esc.
  await key("A");
  await page.send("Input.imeSetComposition", {
    text: "ni",
    selectionStart: 2,
    selectionEnd: 2,
  });
  await sleep(120);
  const composing = await page.evaluate<boolean>(
    "Boolean(document.querySelector('.cm-content')?.matches(':focus-within, :focus'))",
  );
  await key(" ", 0, { text: undefined });
  check(
    await mode() !== "command",
    "Space during composition does not arm the leader",
  );
  await page.send("Input.insertText", { text: "你" });
  await sleep(150);
  check(
    (await editorText()).includes("你"),
    `Composition commits (got ${JSON.stringify(await editorText())})`,
  );
  results.push(
    `IME (CDP imeSetComposition${
      composing ? "" : ", focus not confirmed"
    }): Space during composition stays with the editor; commit lands`,
  );

  // 4. ⌘K from Insert arms the same leader.
  await prefix();
  check(await mode() === "command", "Mod+K arms the leader from Insert");
  await key("Escape");
  await key("Escape");
  results.push(`${mac ? "⌘K" : "Alt+K"} arms the leader from Insert`);

  // 5. ' labels the focused list; a label moves the cursor.
  await click("[data-desktop-item='beta']");
  await key("'");
  await until(
    "document.querySelectorAll('[data-desktop-hint]').length === 5",
    "five row labels",
  );
  await shot("2-list-labels");
  const label = await page.evaluate<string>(`(() => {
    const target = document.querySelector("[data-desktop-item='delta']").getBoundingClientRect();
    return [...document.querySelectorAll("[data-desktop-hint]")].find((hint) => {
      const rect = hint.getBoundingClientRect();
      return rect.top >= target.top && rect.bottom <= target.bottom;
    })?.dataset.desktopHint;
  })()`);
  check(label, "Delta carries a label");
  await key(label);
  check(
    await page.evaluate<boolean>(
      "document.activeElement?.dataset.desktopItem === 'delta' && !document.querySelector('[data-desktop-hint]')",
    ),
    "The label moves the cursor to Delta and clears the labels",
  );
  results.push(`' labels five rows; trusted label "${label}" moves the cursor`);

  // 6. Leader inside a dialog labels its controls.
  await key(" ");
  await key("n");
  await until(
    "document.querySelector('[role=tab][aria-label=Draft]')",
    "␣N opens Create",
  );
  await sleep(400);
  await prefix();
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"modal\"]') && document.querySelector('[data-desktop-hint]')",
    "dialog labels",
  );
  await sleep(250);
  await shot("3-dialog-labels");
  const draft = await page.evaluate<string>(
    `[...document.querySelectorAll("[data-modal-leader-entry]")].find((entry) => entry.textContent.endsWith("Draft"))?.dataset.modalLeaderEntry`,
  );
  check(draft, "The Draft tab has a dialog label");
  await key(draft);
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[role=tab][aria-label=Draft]')?.getAttribute('aria-selected') === 'true'",
    ),
    "The dialog label selects Draft",
  );
  await key("Escape");
  await key("Escape");
  await until(
    "!document.querySelector('[role=tab][aria-label=Draft]')",
    "Create closes with Esc Esc",
  );
  results.push(
    `Dialog leader labels Create's controls; trusted "${draft}" selects Draft; Esc Esc closes`,
  );

  console.log(
    JSON.stringify(
      {
        ok: true,
        browser: page.browser,
        platform: mac ? "macOS" : "other",
        fixture_sha256: page.digest,
        screenshots: output,
        tests: results,
      },
      null,
      2,
    ),
  );
} catch (error) {
  await shot("failure").catch(() => {});
  const errors = await page.evaluate<string[]>("globalThis.__errors ?? []")
    .catch(() => []);
  console.log(
    JSON.stringify(
      {
        ok: false,
        browser: page.browser,
        error: String(error),
        page_errors: errors,
        passed: results,
      },
      null,
      2,
    ),
  );
  Deno.exitCode = 1;
} finally {
  await page.close();
}
