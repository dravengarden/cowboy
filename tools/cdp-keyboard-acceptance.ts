/** Trusted-input Desktop keyboard acceptance in a running Chrome.
 *
 * Unlike the conformance fixtures, every key here is a browser-level
 * `Input.dispatchKeyEvent` (isTrusted), so the page sees what a physical
 * keyboard produces, including the platform modifier. Screenshots of the
 * leader, label and dialog layers are written to the output directory.
 * IME composition uses `Input.imeSetComposition`; it exercises the page's
 * composition path but is not a substitute for a real OS input method.
 *
 * Usage: bun tools/cdp-keyboard-acceptance.ts http://127.0.0.1:9223 <out-dir> [composer|draft|approval]
 */
import { mkdir, writeFile } from "node:fs/promises";
import { type CdpParams, openFixturePage } from "./cdp-fixture.ts";

const endpoint = process.argv.slice(2)[0] ?? "";
const output = process.argv.slice(2)[1];
if (!output?.startsWith("/")) {
  throw new Error("expected an absolute output directory");
}
await mkdir(output, { recursive: true });
const flow = process.argv.slice(2)[2] ?? "composer";
if (
  flow !== "composer" && flow !== "draft" && flow !== "approval" &&
  flow !== "tabs"
) {
  throw new Error("unknown flow");
}

const page = await openFixturePage(
  endpoint,
  "keyboard-acceptance",
  () =>
    `<!doctype html><script type="module">
addEventListener("error", (e) => (globalThis.__errors ??= []).push(String(e.message)));
{ const report = console.error; console.error = (...a) => { (globalThis.__errors ??= []).push(a.map(String).join(" ").slice(0, 600)); report(...a); }; }
addEventListener("unhandledrejection", (e) => (globalThis.__errors ??= []).push(String(e.reason?.stack ?? e.reason)));
${flow === "draft" ? 'localStorage.setItem("cowboy:vim", "1");' : ""}
const fixture = await import("/fixture.js");
await fixture.${
      flow === "draft"
        ? "mountDraftKeyboardAcceptance"
        : flow === "approval"
        ? "mountApprovalKeyboardAcceptance"
        : flow === "tabs"
        ? "mountTabsKeyboardAcceptance"
        : "mountDesktopKeyboardAcceptance"
    }();
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
  "ArrowUp": ["ArrowUp", 38],
  "[": ["BracketLeft", 219],
  "]": ["BracketRight", 221],
  "<": ["Comma", 188],
  ">": ["Period", 190],
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
  await writeFile(
    `${output}/${name}.png`,
    Uint8Array.from(atob(data), (c) => c.charCodeAt(0)),
  );
};
const mode = () => page.evaluate<string>("globalThis.__workspace?.mode");
const editorText = () =>
  page.evaluate<string>(
    "globalThis.__acceptance.editor.current?.getValue() ?? ''",
  );

const draftFlow = async (): Promise<void> => {
  const title = "document.querySelector(\"input[aria-label='Draft title']\")";
  const titleFocused = () =>
    page.evaluate<boolean>(`document.activeElement === ${title}`);
  const inBody = () =>
    page.evaluate<boolean>(
      "Boolean(document.activeElement?.closest('.cm-editor'))",
    );
  await until(
    `${title} && document.querySelector('[data-workspace-document] .cm-content')`,
    "Draft page mounts",
    20_000,
  );
  await sleep(800);
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-draft-title-shortcut]')?.textContent.includes('␣T')",
    ),
    "The title shows its ␣T slot",
  );
  // Write two lines in the shared Vim editor.
  await click("[data-workspace-document] .cm-content");
  await key("i");
  await page.send("Input.insertText", { text: "first line" });
  await key("Enter");
  await page.send("Input.insertText", { text: "second line" });
  await key("Escape");
  await sleep(150);
  // Vim Normal: gg then k on the first line enters the title at its end.
  await key("g");
  await key("g");
  await key("k");
  await sleep(150);
  check(await titleFocused(), "Vim k on the first body line enters the title");
  check(
    await page.evaluate<boolean>(
      `${title}.dataset.vimInputMode === "normal" && ${title}.selectionEnd === ${title}.value.length`,
    ),
    "The title takes a Vim Normal block cursor on its last character",
  );
  results.push(
    "Vim Normal gg then k on the first line enters the title in Normal at its end",
  );
  await key("A", 8);
  await page.send("Input.insertText", { text: " renamed" });
  await key("Enter");
  await sleep(150);
  check(await inBody(), "Enter in the title returns to the body");
  check(
    await page.evaluate<boolean>(
      `${title}.value === "Acceptance draft renamed"`,
    ),
    "Typing in the title renames the draft",
  );
  results.push("Typing renames; Enter returns to the body start");
  // Insert-mode ArrowUp on the first line also enters the title.
  await key("ArrowUp");
  await sleep(150);
  check(
    await page.evaluate<boolean>(
      `document.activeElement === ${title} && ${title}.dataset.vimInputMode === "insert"`,
    ),
    "Insert ↑ on the first line enters the title typing",
  );
  await key("Escape");
  await sleep(100);
  check(
    await page.evaluate<boolean>(`${title}.dataset.vimInputMode === "normal"`),
    "Esc leaves the title's Insert for its Normal",
  );
  await key("Escape");
  await sleep(150);
  check(await inBody(), "A second Esc returns to the body");
  results.push(
    "Insert ↑ enters the title typing; Esc to its Normal, Esc again to the body",
  );
  // On the Draft page its own actions are root keys: which-key lists them
  // under Here, and Session-only keys (Conversation, Plan, Queue) are absent.
  await key("Escape");
  await key(" ");
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"root\"]')",
    "␣ opens which-key on the Draft page",
  );
  await sleep(250);
  await shot("4-draft-leader");
  check(
    await page.evaluate<boolean>(
      "['t','y','h','e'].every((k) => document.querySelector(`[data-leader-entry=\"${k}\"]`)) && !['c','l','q','d'].some((k) => document.querySelector(`[data-leader-entry=\"${k}\"]`))",
    ),
    "Draft which-key lists T Y H E and no Session-only keys",
  );
  await key("t");
  await sleep(150);
  check(
    await page.evaluate<boolean>(
      `document.activeElement === ${title} && ${title}.dataset.vimInputMode === "normal" && ${title}.selectionEnd === ${title}.value.length && ${title}.selectionStart === ${title}.value.length - 1`,
    ),
    "␣T puts a Vim Normal block cursor on the title's last character",
  );
  // The title is a Vim field: b moves by word, A appends, Esc returns to
  // Normal, j returns to the body.
  await key("b");
  check(
    await page.evaluate<boolean>(
      `${title}.selectionStart === ${title}.value.lastIndexOf(" ") + 1`,
    ),
    "b moves to the title's last word",
  );
  await shot("4b-title-normal");
  await key("A", 8);
  check(
    await page.evaluate<boolean>(`${title}.dataset.vimInputMode === "insert"`),
    "A types at the end of the title",
  );
  await key("Escape");
  check(
    await page.evaluate<boolean>(`${title}.dataset.vimInputMode === "normal"`),
    "Esc returns the title to Normal",
  );
  await key("j");
  await sleep(150);
  check(await inBody(), "j in the title's Normal returns to the body");
  results.push(
    "Draft which-key: T Y H E at the root, no Session-only keys; ␣T puts a Vim Normal cursor on the title; b, A, Esc, j work there",
  );
  await key("Escape");
  await sleep(150);
  await shot("5-draft-page");
};

const approvalFlow = async (): Promise<void> => {
  const choice = (n: number) =>
    `document.querySelector('[data-desktop-approval-choice="${n}"]')`;
  const chosen = (n: number) =>
    page.evaluate<boolean>(
      `${choice(n)}?.disabled === true && ${
        choice(n)
      }.querySelector('.MuiCircularProgress-root') !== null`,
    );
  const subject = () =>
    page.evaluate<string>(
      "document.querySelector('[data-desktop-approval-subject]')?.textContent ?? ''",
    );
  const resolve = async () => {
    await page.evaluate("globalThis.__approval.resolve()");
    await sleep(200);
  };
  await until(
    "globalThis.__workspace && document.querySelector('[data-desktop-approval-dock]') && document.querySelector('.cm-content')",
    "approval surface mounted",
    20_000,
  );
  await sleep(500);

  // 1. The dock numbers every choice and counts the queue.
  check(
    await page.evaluate<boolean>(
      `[1, 2, 3].every((n) => document.querySelector('[data-desktop-approval-choice="' + n + '"] kbd')?.textContent === '␣' + n)`,
    ),
    "Every choice carries its ␣N slot",
  );
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-desktop-approval-waiting]')?.textContent === '1 of 3 waiting'",
    ),
    "The header counts the waiting requests",
  );
  check(
    (await subject()).includes("scripts/mp check"),
    "The oldest request is first",
  );
  await shot("approval-dock");
  results.push(
    "The dock shows the oldest request, its numbered choices and the queue",
  );

  // 2. Reading history neither moves nor collapses the dock.
  const top = () =>
    page.evaluate<number>(
      "document.querySelector('[data-desktop-approval-dock]').getBoundingClientRect().top",
    );
  const before = await top();
  await page.evaluate(
    "document.querySelector('[data-approval-transcript]').scrollTop = 0",
  );
  await sleep(150);
  check(
    await top() === before &&
      await page.evaluate<boolean>(`${choice(1)} !== null`),
    "Scrolling the transcript leaves the dock and its buttons in place",
  );
  results.push("Scrolling the transcript neither moves nor collapses the dock");

  // 3. From the reader: Space arms the leader, which-key names the choices
  // by the Provider's labels, and the digit answers.
  await click("[data-approval-transcript]");
  await key(" ");
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"root\"]')",
    "which-key opens",
  );
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-desktop-leader-menu]').textContent.includes(\"Yes, and don't ask again\")",
    ),
    "which-key lists the choices by their own labels",
  );
  await shot("approval-leader");
  await key("1");
  await sleep(200);
  check(await chosen(1), "␣1 answers with the first choice");
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-desktop-approval-dock] [role=\"alert\"]')?.textContent.startsWith('Not connected')",
    ),
    "An answer that cannot leave says so",
  );
  await shot("approval-answered");
  results.push(
    "Reader: ␣1 answers; the choice spins and reports the closed connection",
  );

  // 4. One answer per request: the other keys wait.
  await key(" ");
  await key("3");
  await sleep(150);
  check(
    await chosen(1) && !(await chosen(3)),
    "A second key cannot change an answer in flight",
  );
  results.push(
    "A request takes one answer; later keys are ignored while it is in flight",
  );

  // 5. Resolution brings the next request, unanswered.
  await resolve();
  check(
    (await subject()) === "git status --short",
    "The next request follows",
  );
  check(
    await page.evaluate<boolean>(
      `${
        choice(1)
      }.disabled === false && document.querySelector('[data-desktop-approval-waiting]')?.textContent === '1 of 2 waiting'`,
    ),
    "The next request starts unanswered",
  );

  // 6. From Vim Insert: the platform prefix then the digit answers, and the
  // digit never reaches the editor.
  await click(".cm-content");
  for (const value of ["i", "h", "i"]) await key(value);
  await prefix();
  await key("3");
  await sleep(200);
  check(await chosen(3), "The prefix then 3 answers from Insert");
  const text = await page.evaluate<string>(
    "globalThis.__approval.editor.current?.getValue() ?? ''",
  );
  check(
    text === "hi",
    `The digit stays out of the editor (got ${JSON.stringify(text)})`,
  );
  results.push(
    `Insert: ${
      mac ? "⌘K" : "Alt+K"
    } then 3 rejects without leaving the editor or typing the digit`,
  );

  // 7. A pointer press on a choice answers too.
  await resolve();
  check((await subject()) === "ls -la", "The last request follows");
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-desktop-approval-waiting]') === null",
    ),
    "A single request shows no queue count",
  );
  await click('[data-desktop-approval-choice="2"]');
  check(await chosen(2), "A click answers with that choice");
  results.push("A pointer press on a choice answers it");

  // 8. With nothing waiting the dock and its keys are gone.
  await resolve();
  check(
    await page.evaluate<boolean>(
      "document.querySelector('[data-desktop-approval-dock]') === null",
    ),
    "The dock leaves with the last request",
  );
  await key("Escape");
  await key(" ");
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"root\"]')",
    "which-key opens without an approval",
  );
  check(
    await page.evaluate<boolean>(
      "!document.querySelector('[data-desktop-leader-menu]').textContent.includes('Yes')",
    ),
    "which-key no longer offers the choices",
  );
  await key("Escape");
  results.push("Nothing waiting: the dock and its leader keys are gone");
};

const tabsFlow = async (): Promise<void> => {
  // `^` pinned, `?` preview, `*` selected, in strip order.
  const strip = () =>
    page.evaluate<string>(`[...document.querySelectorAll('[data-desktop-tab]')]
      .map((tab) => (tab.dataset.desktopTabState === 'pinned' ? '^' : '') +
        tab.dataset.desktopTab +
        (tab.dataset.desktopTabState === 'preview' ? '?' : '') +
        (tab.getAttribute('aria-selected') === 'true' ? '*' : ''))
      .join(' ')`);
  const expect = async (expected: string, message: string) => {
    await sleep(120);
    const actual = await strip();
    check(
      actual === expected,
      `${message}: expected "${expected}", got "${actual}"`,
    );
  };
  const tab = (id: string) => `[data-desktop-tab="${id}"]`;
  const browse = (id: string) => click(`[data-acceptance-open="${id}"]`);
  const body = async () => {
    await page.evaluate(
      "document.querySelector('[data-acceptance-body]').focus()",
    );
    await sleep(80);
  };
  const leader = async (...keys: string[]) => {
    await body();
    await key(" ");
    for (const value of keys) {
      await key(value, value === "<" || value === ">" ? 8 : 0);
    }
  };
  const pointer = async (
    selector: string,
    button: "left" | "middle" | "right",
    clickCount = 1,
  ) => {
    const box = await page.evaluate<{ x: number; y: number } | null>(`(() => {
      const element = document.querySelector(${JSON.stringify(selector)});
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { x: rect.left + 30, y: rect.top + rect.height / 2 };
    })()`);
    check(box, `Missing ${selector}`);
    for (const type of ["mousePressed", "mouseReleased"]) {
      await page.send("Input.dispatchMouseEvent", {
        type,
        x: box.x,
        y: box.y,
        button,
        clickCount,
      });
    }
    await sleep(150);
  };
  await until(
    "document.querySelector('[data-desktop-tabs]')",
    "tab strip mounts",
    20_000,
  );
  await expect("alpha?*", "The item on screen has a preview tab");
  const sizes = await page.evaluate<number[]>(
    `[...document.querySelectorAll('[data-desktop-tab] svg')].slice(0, 2).map((icon) => Math.round(icon.getBoundingClientRect().width))`,
  );
  check(
    sizes[0] === 10 && sizes[1] === 16,
    `The status dot keeps its own size beside the 16px glyph: ${
      sizes.join(", ")
    }`,
  );
  await browse("beta");
  await expect("beta?*", "Browsing reuses the preview tab");
  await pointer(tab("beta"), "left");
  await pointer(tab("beta"), "left", 2);
  await expect("beta*", "A double click keeps the tab");
  await browse("gamma");
  await expect("beta gamma?*", "The next item opens right of the current tab");
  await page.evaluate(
    "document.querySelector('[data-acceptance-input]').focus()",
  );
  await page.send("Input.insertText", { text: "typed" });
  await expect("beta gamma*", "Typing in the item keeps its tab");
  await browse("delta");
  await leader("b", "k");
  await expect("beta gamma delta*", "␣BK keeps the preview tab");
  await browse("draft:notes");
  await expect(
    "beta gamma delta draft:notes?*",
    "A Draft takes a tab like a Session",
  );
  results.push(
    "Preview tab: browsing reuses it; double click, typing and ␣BK keep it; Sessions and Drafts share the strip",
  );

  await leader("[");
  await expect("beta gamma delta* draft:notes?", "␣[ opens the previous tab");
  await leader("]");
  await leader("]");
  await expect("beta* gamma delta draft:notes?", "␣] wraps to the first tab");
  check(
    await page.evaluate<boolean>(
      `document.querySelector('${
        tab("gamma")
      } kbd')?.textContent?.trim() === "␣2"`,
    ),
    "Every tab shows its leader digit",
  );
  await body();
  await key(" ");
  await sleep(350);
  await shot("6-tabs-leader");
  await key("3");
  await expect("beta gamma delta* draft:notes?", "␣3 opens the third tab");
  results.push(
    "␣[ ␣] step and wrap; every tab shows its digit and ␣3 opens the third",
  );

  await leader("b", "p");
  await expect("^delta* beta gamma draft:notes?", "␣BP pins to the front");
  await leader("2");
  await leader("b", "r");
  await expect("^delta beta*", "␣BR closes the tabs to the right");
  await leader("b", "u");
  await expect("^delta beta draft:notes*", "␣BU reopens the last closed tab");
  await leader("b", "u");
  await expect(
    "^delta beta draft:notes gamma*",
    "␣BU again reopens the one before it",
  );
  await leader("b", "<");
  await expect("^delta beta gamma* draft:notes", "␣B< moves the tab left");
  await leader("b", "d");
  await expect(
    "^delta beta draft:notes*",
    "␣BD closes the tab and returns to the last visited one",
  );
  await leader("b", "o");
  await expect("^delta draft:notes*", "␣BO spares pinned tabs");
  results.push(
    "␣BP pins; ␣BR ␣BD ␣BO close (pinned tabs stay); ␣BU reopens; ␣B< moves",
  );

  await browse("alpha");
  await browse("beta");
  await expect("^delta draft:notes beta?*", "The preview tab is reused again");
  await pointer(tab("draft:notes"), "middle");
  await expect("^delta beta?*", "A middle click closes a tab");
  await pointer(tab("delta"), "right");
  await until("document.querySelector('[role=menu]')", "context menu opens");
  const menu = await page.evaluate<string>(
    "[...document.querySelectorAll('[role=menuitem]')].map((item) => item.textContent.trim().split('␣')[0] + (item.getAttribute('aria-disabled') === 'true' ? ' (off)' : '')).join(' | ')",
  );
  check(
    menu ===
      "Close | Close Others | Close to the Right | Close to the Left (off) | Close All | Unpin | Reopen Closed Tab | Rename…",
    `Context menu of a pinned Session tab: ${menu}`,
  );
  await shot("7-tabs-menu");
  await page.evaluate(
    "[...document.querySelectorAll('[role=menuitem]')].find((item) => item.textContent.startsWith('Unpin')).click()",
  );
  await expect("delta beta?*", "The menu unpins the tab it was opened on");
  await pointer(tab("delta"), "right");
  await until("document.querySelector('[role=menu]')", "context menu opens");
  await page.evaluate(
    "[...document.querySelectorAll('[role=menuitem]')].find((item) => item.textContent.startsWith('Close All')).click()",
  );
  await expect("beta?*", "Close All leaves the item on screen its tab");
  results.push(
    "Pointer: middle click closes; the context menu acts on its own tab; Close All keeps the current item",
  );
  await shot("8-tabs");
};

try {
  if (flow === "tabs") {
    await tabsFlow();
    console.log(
      JSON.stringify(
        {
          ok: true,
          browser: page.browser,
          flow,
          screenshots: output,
          tests: results,
        },
        null,
        2,
      ),
    );
  } else if (flow === "draft") {
    await draftFlow();
    console.log(
      JSON.stringify(
        {
          ok: true,
          browser: page.browser,
          flow,
          screenshots: output,
          tests: results,
        },
        null,
        2,
      ),
    );
  } else if (flow === "approval") {
    await approvalFlow();
    console.log(
      JSON.stringify(
        {
          ok: true,
          browser: page.browser,
          flow,
          screenshots: output,
          tests: results,
        },
        null,
        2,
      ),
    );
  } else {
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
    results.push(
      "Insert: trusted Space types a space and never arms the leader",
    );

    // 1b. A Session switch may focus the editable twice within CodeMirror's
    // 10ms focus-report window. Normal must still hand
    // focus to the non-editable sink, or the first `i` reaches the OS IME.
    await key("Escape");
    await page.evaluate(`new Promise((resolve) => {
      const content = () => document.querySelector(".cm-content").focus();
      content();
      setTimeout(() => { content(); setTimeout(resolve, 200); }, 14);
    })`);
    check(
      await page.evaluate<boolean>(
        "document.activeElement?.matches('[data-vim-command-sink]') === true",
      ),
      "A repeated Prompt focus in Normal settles on the Vim command sink",
    );
    await page.evaluate(`document.addEventListener("keydown", (event) => {
      globalThis.__firstKeyTarget = event.target.matches?.("[data-vim-command-sink]") ? "sink" : "editable";
    }, { capture: true, once: true })`);
    await key("i");
    check(
      await page.evaluate<string>("globalThis.__firstKeyTarget") === "sink",
      "The first Normal `i` after a repeated focus reaches the command sink",
    );
    results.push(
      "Repeated Prompt focus in Normal: focus settles on the Vim sink; the first `i` never reaches the editable",
    );

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
    await sleep(250); // let the which-key entrance animation finish
    await shot("1-leader-which-key");
    await key("Escape");
    check(await mode() === "normal", "Esc closes the leader");
    check(
      !(await editorText()).includes("  "),
      "The leader Space typed nothing",
    );
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
    await key("A", 8);
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
    results.push(
      `' labels five rows; trusted label "${label}" moves the cursor`,
    );

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
    // The modal grammar with trusted keys: Esc leaves the title for Normal,
    // digits pick the type, j/k walk rows, Esc closes.
    await key("Escape");
    await until(
      "document.activeElement?.hasAttribute('data-desktop-field-cursor')",
      "Esc puts the Normal cursor on the title",
    );
    check(
      await page.evaluate<boolean>(
        "document.querySelectorAll(\"[role=tab] [data-shortcut-state='available']\").length === 3",
      ),
      "Normal lights the 1-3 type digits",
    );
    await sleep(200);
    await shot("3b-create-normal");
    await key("3");
    check(
      await page.evaluate<boolean>(
        "document.querySelector('[role=tab][aria-label=Folder]')?.getAttribute('aria-selected') === 'true' && document.activeElement?.hasAttribute('data-desktop-field-cursor')",
      ),
      "3 picks Folder and keeps the cursor on the title",
    );
    await key("k");
    check(
      await page.evaluate<boolean>(
        "document.activeElement?.getAttribute('aria-label') === 'Folder'",
      ),
      "k reaches the selected tab",
    );
    await key("h");
    check(
      await page.evaluate<boolean>(
        "document.querySelector('[role=tab][aria-label=Draft]')?.getAttribute('aria-selected') === 'true'",
      ),
      "h on the tabs selects Draft",
    );
    await key("Escape");
    await until(
      "!document.querySelector('[role=tab][aria-label=Draft]')",
      "Esc in Normal closes Create",
    );
    results.push(
      `Dialog leader labels Create's controls; trusted "${draft}" selects Draft; Esc to Normal, 3/k/h move, Esc closes`,
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
  }
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
  process.exitCode = 1;
} finally {
  await page.close();
}
