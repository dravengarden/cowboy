// The `surfaces` flow of tools/cdp-keyboard-acceptance.ts: the real Desktop
// App with trusted keys, for the Composer's surfaces and the work pane's
// Conversation/Review views.
import type { AcceptanceDriver } from "./cdp-run-config-flow.ts";

// Composer surfaces in the real Desktop App: a trusted Escape reaches each
// surface through the Desktop key dispatcher, as a physical key does.
export async function surfacesFlow(
  driver: AcceptanceDriver,
  click: (selector: string) => Promise<void>,
): Promise<void> {
  const { page, key, until, check, shot, results, mac } = driver;
  const prefix = () => key("k", mac ? 4 : 1);
  const mounted = await page.evaluate<string>(
    `import("/fixture.js").then((m) => m.mountComposerSurfacesKeyboardAcceptance()).then(() => "mounted", (e) => String(e && e.stack || e))`,
  );
  check(mounted === "mounted", `surfaces fixture mounts: ${mounted}`);
  // The Prompt editor is a textarea or a contenteditable surface.
  const editor =
    "[data-desktop-pane='prompt'] textarea, [data-desktop-pane='prompt'] [contenteditable=true]";
  const editorText = `(() => { const e = document.querySelector(${JSON.stringify(editor)}); return e.value ?? e.textContent; })()`;
  await until(`document.querySelector(${JSON.stringify(editor)})`, "composer editor", 20_000);
  await page.evaluate(`(() => {
    const editor = document.querySelector(${JSON.stringify(editor)});
    editor.focus();
    document.execCommand("selectAll");
    document.execCommand("insertText", false, "surface keys");
  })()`);
  const enabled = (id: string) => `[data-composer-action=${id}]:not([disabled])`;
  await until(`document.querySelector(${JSON.stringify(enabled("schedule"))})`, "schedule enabled");
  await click(enabled("schedule"));
  await until(`document.querySelector("[aria-label=发送时间]")`, "schedule sheet opens");
  await key("Escape");
  await until(
    `!document.querySelector("[aria-label=发送时间]")`,
    "Escape closes the schedule sheet",
  );
  results.push("Schedule sheet: a trusted Escape closes it");

  await click(enabled("send"));
  await until(`document.querySelector("[aria-label='Choose delivery']")`, "delivery picker opens");
  await shot("surfaces-1-picker");
  await key("Escape");
  await until(
    `!document.querySelector("[aria-label='Choose delivery']")`,
    "Escape closes the delivery picker",
  );
  await until(
    `document.activeElement === document.querySelector(${JSON.stringify(editor)}) || document.querySelector(${JSON.stringify(editor)})?.contains(document.activeElement)`,
    "focus returns to the Prompt editor after the picker closes",
  );
  check(
    (await page.evaluate<string>(editorText)).includes("surface keys"),
    "closing the picker keeps the draft",
  );
  results.push(
    "Delivery picker: a trusted Escape closes it, returns focus to the Prompt editor and keeps the draft",
  );

  await click(enabled("draft"));
  const move = "[aria-label='Move to another session…']";
  await until(`document.querySelector(${JSON.stringify(move)})`, "saved draft offers Move");
  // The inline action collapses into the row kebab on a narrow panel, so it
  // may have no box to aim a pointer at; Escape below stays trusted.
  await page.evaluate(`document.querySelector(${JSON.stringify(move)}).click()`);
  const destination = `[...document.querySelectorAll("[role=dialog] *")].some((node) => node.textContent?.trim() === "Destination session")`;
  await until(destination, "move picker opens");
  await shot("surfaces-2-move");
  await key("Escape");
  await until(`!(${destination})`, "Escape closes the move picker");
  check(
    !(await page.evaluate<boolean>(
      `globalThis.__surfaceSent.some((frame) => frame.type === "move_draft")`,
    )),
    "dismissing the move picker moves nothing",
  );
  results.push("Move-draft picker: a trusted Escape closes it and moves nothing");

  // Code Review is a view of the work pane (docs/desktop-code-review.md):
  // ␣V shows it, Escape never leaves it, ␣V V focuses it and ␣C returns.
  const reviewShown = `document.querySelector("[data-desktop-review]:not([hidden])")`;
  const focusIn = (region: string) =>
    `document.activeElement?.closest("[data-desktop-region='${region}']")`;
  const statusPane = `[...document.querySelectorAll("[data-desktop-status-line] span")].map((s) => s.textContent).join(" ")`;
  await click("[data-desktop-region='conversation.transcript']");
  await key(" ");
  await key("v");
  await until(reviewShown, "␣V shows Review");
  // Review owns focus in either layout: the workbench's Document region, or
  // the classic layout's single region.
  const inReview = `document.activeElement?.closest("[data-desktop-review]")`;
  await until(inReview, "␣V focuses Review");
  check(
    await page.evaluate<boolean>(
      `document.querySelector("[data-desktop-work-view='review']").getAttribute("aria-selected") === "true"`,
    ),
    "the Review view tab is selected",
  );
  check((await page.evaluate<string>(statusPane)).includes("REVIEW"), "the status line names Review");
  await shot("surfaces-3-review");
  for (let i = 0; i < 3; i++) await key("Escape");
  check(await page.evaluate<boolean>(`Boolean(${reviewShown})`), "Escape never leaves Review");
  results.push("Review: ␣V shows and focuses it, its tab and status segment say so, Escape keeps it");

  await click(editor);
  await prefix();
  await key("v");
  await until(`document.querySelector('[data-desktop-leader-menu="group:v"]')`, "␣V opens the Review group while Review is shown");
  await shot("surfaces-4-review-group");
  await key("v");
  await until(inReview, "␣V V focuses Review");
  results.push(`Review group: ${mac ? "⌘K" : "Alt+K"} V opens it from the Prompt, V focuses Review`);

  await key(" ");
  await key("c");
  await until(`!${reviewShown}`, "␣C returns to the Conversation");
  await until(focusIn("conversation.transcript"), "␣C focuses the transcript");
  check(
    await page.evaluate<boolean>(`Boolean(document.querySelector("[data-desktop-review][hidden]"))`),
    "Review stays mounted behind the Conversation",
  );
  results.push("␣C returns to the same transcript and keeps Review mounted behind it");
}

