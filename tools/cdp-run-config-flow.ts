/** The Run configuration flow of tools/cdp-keyboard-acceptance.ts: the
 * full-window workbench driven with trusted keys over the real Top bar,
 * store and command host (keyboardAcceptanceFixture.tsx). */
import type { FixturePage } from "./cdp-fixture.ts";

export interface AcceptanceDriver {
  page: FixturePage;
  key: (value: string, modifiers?: number) => Promise<void>;
  until: (expression: string, message: string, ms?: number) => Promise<void>;
  sleep: (ms: number) => Promise<unknown>;
  check: (value: unknown, message: string) => void;
  shot: (name: string) => Promise<void>;
  results: string[];
  mac: boolean;
}

function helpers({ page }: AcceptanceDriver) {
  const viewport = (width: number, height: number) =>
    page.send("Emulation.setDeviceMetricsOverride", {
      width,
      height,
      deviceScaleFactor: 1,
      mobile: false,
    });
  const activeIn = (selector: string) =>
    page.evaluate<boolean>(
      `Boolean(document.activeElement?.closest(${JSON.stringify(selector)}))`,
    );
  const sent = () =>
    page.evaluate<string[]>("[...globalThis.__acceptance.sent]");
  const sentSince = async (from: number) => (await sent()).slice(from);
  return { viewport, activeIn, sent, sentSince };
}

/** Opens on the active preset with Agent over Tools and Context beside them. */
async function openWorkbench(driver: AcceptanceDriver): Promise<void> {
  const { page, until, sleep, check, shot, results } = driver;
  const { viewport } = helpers(driver);
  await viewport(1600, 1000);
  await until(
    "document.querySelector(\"[data-config-pane='tools'] [data-settings-row]\") && " +
      "document.querySelector(\"[data-config-pane='context'] [data-context-links]\") && " +
      "document.querySelectorAll('[data-config-preset]').length === 6",
    "Run configuration opens with every pane",
    20_000,
  ).catch(async (error: unknown) => {
    await shot("run-config-failed-open");
    const body = await page.evaluate<string>(
      "document.body.innerText.slice(0, 600) + ' | panes=' + document.querySelectorAll('[data-config-pane]').length + ' presets=' + document.querySelectorAll('[data-config-preset]').length + ' tools=' + document.querySelectorAll('[data-settings-row]').length + ' links=' + document.querySelectorAll('[data-context-links]').length + ' requests=' + JSON.stringify(globalThis.__acceptance?.requests)",
    );
    throw new Error(`${String(error)}: ${body}`);
  });
  await sleep(500);
  const layout = await page.evaluate<
    { panes: number[][]; paper: number[]; view: number[] }
  >(`(() => {
    const box = (e) => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map(Math.round); };
    return {
      panes: [...document.querySelectorAll("[data-config-pane]")].map(box),
      paper: box(document.querySelector("[data-run-config-panel]").closest(".MuiDialog-paper")),
      view: [document.documentElement.clientWidth, document.documentElement.clientHeight],
    };
  })()`);
  check(
    layout.paper[2]! >= layout.view[0]! - 48 - 2 &&
      layout.paper[3]! >= layout.view[1]! - 48 - 2,
    `The workbench fills the window: ${JSON.stringify(layout)}`,
  );
  const [agentBox, toolsBox, contextBox] = layout.panes as [
    number[],
    number[],
    number[],
  ];
  check(
    layout.panes.length === 3 && agentBox[0] === toolsBox[0] &&
      toolsBox[1]! > agentBox[1]! && contextBox[0]! > agentBox[0]! &&
      contextBox[3]! >= layout.view[1]! - 220,
    `Agent over Tools on the left, a full-height Context on the right: ${
      JSON.stringify(layout.panes)
    }`,
  );
  results.push("Agent stacks over Tools; Context fills the right column");
  check(
    await page.evaluate<boolean>(
      "document.activeElement?.getAttribute('data-config-preset') === '2'",
    ),
    "The cursor opens on the active preset",
  );
  await shot("run-config-wide");
}

/** Agent edits stage in a draft until Mod+Enter applies them. */
async function agentDraft(driver: AcceptanceDriver): Promise<void> {
  const { page, key, until, sleep, check, shot, results, mac } = driver;
  const { activeIn, sent, sentSince } = helpers(driver);
  const mark = (await sent()).length;
  const pressed = (index: number) =>
    page.evaluate<boolean>(
      `document.querySelector("[data-config-preset='${
        String(index)
      }']")?.getAttribute("aria-pressed") === "true"`,
    );
  await key("6");
  await sleep(200);
  check(
    await pressed(5) && (await sentSince(mark)).length === 0 &&
      await page.evaluate<boolean>(
        "Boolean(document.querySelector('[data-config-apply-bar]'))",
      ),
    "6 stages the sixth preset without touching the agent",
  );
  results.push("Digit 6 stages the sixth preset (4–6 were dead before)");
  await key("j");
  check(await activeIn("[data-config-shortcut='a']"), "J moves to Agent mode");
  await key("h");
  await sleep(150);
  check(
    await page.evaluate<boolean>(
      "document.querySelector(\"[data-config-shortcut='a'] [aria-pressed='true']\")?.textContent === 'Auto'",
    ),
    "H steps Agent mode left in the draft",
  );
  await key("j");
  check(await activeIn("[data-config-shortcut='m']"), "J moves to Model");
  // Fable (staged) → Opus (the agent's, so no longer a change) → Sonnet.
  await key("h");
  await key("h");
  await sleep(150);
  check(
    !(await page.evaluate<boolean>(
      "Boolean(document.querySelector(\"[role='listbox']\"))",
    )) &&
      await page.evaluate<boolean>(
        "document.querySelector(\"[data-config-shortcut='m']\")?.hasAttribute('data-config-changed') && document.querySelector(\"[data-config-shortcut='m']\").textContent.includes('was Opus')",
      ),
    "H steps Model in place without opening its list, marked unsaved",
  );
  check(
    (await sentSince(mark)).length === 0,
    "Nothing reaches the agent before Apply",
  );
  await shot("run-config-unsaved");
  await key("Enter", mac ? 4 : 2);
  await until(
    "!document.querySelector('[data-config-apply-bar]')",
    "Mod+Enter applies the draft",
  );
  const applied = await sentSince(mark);
  check(
    applied.includes("model=sonnet") &&
      applied.includes("reasoning_effort=max") &&
      applied.includes("mode=auto") &&
      applied.indexOf("model=sonnet") <
        applied.indexOf("reasoning_effort=max"),
    `Apply sends every change once, the model first: ${
      JSON.stringify(applied)
    }`,
  );
  results.push(
    "Edits stage until Mod+Enter applies them together, model first",
  );
  results.push("J/K walk fields; H/L apply the neighbouring value, Model too");
  await key("G", 8);
  check(
    await activeIn("[data-config-shortcut='f']"),
    "G reaches the last field",
  );
  await key("g");
  await key("g");
  check(await activeIn("[data-config-preset]"), "gg returns to the presets");
  results.push("gg/G jump to the pane's first and last field");
}

/** Tools fields are reachable and switches follow H/L. */
async function toolsPane(driver: AcceptanceDriver): Promise<void> {
  const { page, key, until, check, shot, results } = driver;
  const { activeIn } = helpers(driver);
  await key("]");
  check(await activeIn("[data-config-pane='tools']"), "] moves to Tools");
  await shot("run-config-tools-cursor");
  for (let i = 0; i < 12; i++) {
    if (
      await page.evaluate<boolean>(
        "document.activeElement?.getAttribute('aria-label') === 'Allow agent calls'",
      )
    ) break;
    await key("j");
  }
  check(
    await page.evaluate<boolean>(
      "document.activeElement?.getAttribute('aria-label') === 'Allow agent calls'",
    ),
    "J reaches the Allow calls switch in Tools",
  );
  const allow =
    "document.querySelector(\"input[aria-label='Allow agent calls']\")";
  await key("h");
  await until(`!${allow}.checked`, "H turns a switch off");
  await until(`!${allow}.disabled`, "The switch settles after its save");
  const describe = () =>
    page.evaluate<string>(
      "(() => { const a = document.activeElement; return a ? a.tagName + ' ' + (a.getAttribute('aria-label') ?? a.textContent.slice(0, 60)) + ' pane=' + a.closest('[data-config-pane]')?.dataset.configPane : 'none'; })()",
    );
  const before = await describe();
  await key("l");
  await until(`${allow}.checked`, "L turns a switch on").catch(
    async (error: unknown) => {
      throw new Error(
        `${String(error)}; before=${before}; after=${await describe()}`,
      );
    },
  );
  await until(
    `!${allow}.disabled && document.activeElement === ${allow}`,
    "The cursor returns to the switch after its save",
  );
  results.push("Tools fields are reachable; H/L turn a switch off and on");
}

/** The inline session tree, pane motions and the dialog leader. */
async function contextPane(driver: AcceptanceDriver): Promise<void> {
  const { page, key, until, check, shot, results } = driver;
  const { activeIn } = helpers(driver);
  await key("l", 2);
  check(
    await activeIn("[data-config-pane='context']"),
    "Ctrl+L moves to the Context pane",
  );
  // The session tree is inline: reach it with J, toggle a row in place.
  for (let i = 0; i < 6 && !(await activeIn("[role='tree']")); i++) {
    await key("j");
  }
  if (!(await activeIn("[role='tree']"))) {
    await shot("run-config-tree-miss");
    throw new Error(
      `J reaches the inline session tree: ${await page.evaluate<string>(
        "(() => { const a = document.activeElement; return a?.tagName + ' ' + (a?.getAttribute('aria-label') ?? a?.textContent?.slice(0, 80)) + ' trees=' + document.querySelectorAll(\"[role='tree']\").length + ' rows=' + document.querySelectorAll(\"[role='treeitem']\").length + ' fields=' + JSON.stringify(globalThis.__runConfigFields('context')); })()",
      )}`,
    );
  }
  const linkedBefore = await page.evaluate<number>(
    "document.querySelectorAll(\"[role='tree'] [aria-selected='true']\").length",
  );
  await key("j");
  await key("Enter");
  await until(
    `document.querySelectorAll("[role='tree'] [aria-selected='true']").length !== ${linkedBefore}`,
    "Enter toggles a session in the tree",
  );
  check(await activeIn("[role='tree']"), "The tree keeps its own J/K/Enter");
  await shot("run-config-context-tree");
  results.push("Link sessions is an inline tree: J reaches it, Enter toggles");
  await key("[");
  check(
    await activeIn("[data-config-pane='tools']"),
    "[ leaves the tree for the previous pane",
  );
  await key("h", 2);
  check(
    await activeIn("[data-config-preset]"),
    "Returning to Agent restores its cursor",
  );
  results.push("Ctrl+H/L and [/] move between panes; each keeps its cursor");
  await key(" ");
  await until(
    "document.querySelector('[data-desktop-leader-menu=\"modal\"]') && document.querySelector('[data-desktop-hint]')",
    "Space labels every control of the workbench",
  );
  await shot("run-config-leader");
  results.push("Space labels every control across all three panes");
}

/** A narrower window, then the guarded close. */
async function resizeAndClose(driver: AcceptanceDriver): Promise<void> {
  const { page, key, until, sleep, check, shot, results } = driver;
  const { viewport, sent, sentSince } = helpers(driver);
  await key("Escape");
  await sleep(150);
  await viewport(1180, 860);
  await sleep(400);
  const medium = await page.evaluate<number[][]>(`
    [...document.querySelectorAll("[data-config-pane]")].map((e) => {
      const r = e.getBoundingClientRect(); return [r.left, r.top].map(Math.round);
    })`);
  check(
    new Set(medium.map((pane) => pane[0])).size === 2,
    `A narrower window keeps two columns: ${JSON.stringify(medium)}`,
  );
  await shot("run-config-medium");
  results.push("A 1180px window keeps the two columns");
  const beforeClose = (await sent()).length;
  await key("g");
  await key("g");
  await key("f");
  await sleep(150);
  await key("Escape");
  await sleep(200);
  check(
    await page.evaluate<boolean>(
      "Boolean(document.querySelector('[data-run-config-panel]')) && document.querySelector('[data-config-apply-bar]')?.textContent.includes('Esc again')",
    ),
    "Esc with unsaved edits keeps the dialog and warns",
  );
  await key("Escape");
  await until(
    "!document.querySelector('[data-run-config-panel]')",
    "A second Esc discards and closes",
  );
  check(
    (await sentSince(beforeClose)).length === 0,
    "Discarded edits never reach the agent",
  );
  results.push("Esc warns about unsaved edits; a second Esc discards them");
  await page.send("Emulation.clearDeviceMetricsOverride", {});
  const errors = await page.evaluate<string[]>("globalThis.__errors ?? []");
  check(errors.length === 0, `No page errors: ${JSON.stringify(errors)}`);
}

export async function runConfigFlow(driver: AcceptanceDriver): Promise<void> {
  for (
    const step of [
      openWorkbench,
      agentDraft,
      toolsPane,
      contextPane,
      resizeAndClose,
    ]
  ) {
    await step(driver);
  }
}
