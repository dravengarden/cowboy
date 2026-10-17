import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

/** Comment-free, whitespace-collapsed view: these are assertions about the
 *  STYLE, and the formatter is free to rewrap any of these template literals. */
function code(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/^\s*\/\/.*$/gm, " ")
    .replace(/\s+/g, " ");
}

const shelfSource = await readFile(
  new URL("./decisionShelf.ts", import.meta.url), "utf8",
);
const shelf = code(shelfSource);
const emphasis = shelf.slice(
  shelf.indexOf("export function decisionActionEmphasis"),
  shelf.indexOf("export function decisionShelfSurface"),
);
const surface = shelf.slice(
  shelf.indexOf("export function decisionShelfSurface"),
);
const actions = await readFile(
  new URL("./MobileDecisionActions.tsx", import.meta.url), "utf8",
);
const card = await readFile(
  new URL("./ObsidianSheet.tsx", import.meta.url), "utf8",
);
const filterSheets = await Promise.all(
  [
    "./UsageLogs.tsx",
    "./ProviderUsageActivityDetails.tsx",
    "./ObservabilityFilters.tsx",
  ].map((file) => readFile(new URL(file, import.meta.url), "utf8")),
);

// The lift is painted, never composited. A wide upward `box-shadow` on this
// strip is the exact thing iOS WebKit turned into a large purple rectangle over
// the frosted cover sheet (`fix(mobile): flatten new session action footer`).
// Gradients have no shadow/backdrop-filter interaction, so they cannot regress
// that way — keep the plate itself shadow-free.
test("the decision plate lifts with a gradient, never an upward shadow", () => {
  assert(surface.includes('bottom: "100%"'));
  assert(surface.includes("linear-gradient(to top"));
  assertEquals(/boxShadow|box-shadow/.test(surface), false);
  // Fades end in a zero-alpha version of their OWN colour: the `transparent`
  // keyword fades through black in Safari.
  assert(surface.includes("alpha(accent, 0)"));
  assert(surface.includes("alpha(shade, 0)"));
  assertEquals(/linear-gradient\([^)]*transparent/.test(surface), false);
});

test("decision feedback preserves stationary targets and semantic button colors", () => {
  assert(shelf.includes("const accent = t.palette.primary.main;"));
  assert(/borderTop: `1px solid \$\{\s*alpha\(accent/.test(surface));
  // State paint inherits each button's foreground; destructive variants keep
  // their MUI palette instead of acquiring an unrelated primary-color glow.
  assert(emphasis.includes('backgroundColor: "currentColor"'));
  // Two choices, two equal columns.
  assert(/"& > \.MuiButton-root": \{ flex: 1,/.test(emphasis));
  assert(emphasis.includes("minHeight: 44"));
  assertEquals(emphasis.includes("translateY"), false);
  assert(emphasis.includes('&:active:not(.Mui-disabled)::after'));
  assert(emphasis.includes("prefers-reduced-motion"));
  // Doubled selectors: `actions` children come from foreign call sites
  // (ConfirmSheet's NetworkButton), so this must outrank MUI's own rules
  // regardless of stylesheet order.
  assert(emphasis.includes(".MuiButton-root.MuiButton-root"));
  assert(emphasis.includes(".MuiButton-text.MuiButton-text"));
});

test("decision surfaces share their default material with an explicit flat form variant", () => {
  // The two real decision FOOTERS get the whole plate.
  assert(actions.includes("decisionShelfSurface(theme)"));
  assert(actions.includes("flatDecisionShelf(theme)"));
  assert(actions.includes("flatDecisionActions(theme)"));
  assert(actions.includes("...decisionActionEmphasis(theme),"));
  assert(
    card.includes("...decisionShelfSurface(theme, { riser: bodyScrolls }),"),
  );
  assert(card.includes("body.scrollHeight > body.clientHeight + 1"));
  assert(card.includes("data-mobile-decision-footer-shelf"));
  assertEquals(
    code(card).includes(
      'pb: SAFE_INSIDE, borderTop: 1, borderColor: "divider"',
    ),
    false,
  );
  // Filter sheets keep their in-body row (the Desktop dialog branch drops
  // `actions`), so they share the BUTTON design without the chrome plate.
  for (const source of filterSheets) {
    assert(source.includes("decisionActionEmphasis"));
    assertEquals(source.includes("decisionShelfSurface"), false);
  }
});

const consequence = await readFile(
  new URL("./ConfirmConsequence.tsx", import.meta.url), "utf8",
);
const composer = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);
const topBar = await readFile(
  new URL("./desktop/DesktopTopBarControls.tsx", import.meta.url), "utf8",
);
const commands = await readFile(
  new URL("./agentCommands.ts", import.meta.url), "utf8",
);

// A confirm card states two different things: what the action MEANS, and what
// happens to THIS session on tap. The second one was a 13px grey orphan between
// the paragraph and the buttons; it is now a block in the decision's colour.
test("the consequence is a block in the decision's colour, not an orphan line", () => {
  assert(consequence.includes("data-confirm-consequence={tone}"));
  assert(consequence.includes("theme.palette[tone].main"));
  assert(consequence.includes("This can't be undone."));
  // Both surfaces render the same block, destructive ones in error red.
  assert(composer.includes("<ConfirmConsequence"));
  assert(composer.includes('tone={action.destructive ? "error" : "primary"}'));
  assert(composer.includes("irreversible={action.destructive}"));
  assert(topBar.includes('<ConfirmConsequence tone="error" irreversible>'));
  // Neither surface keeps the old floating caption.
  for (const source of [composer, topBar]) {
    assertEquals(
      source.includes('sx={{ mt: 1.5, fontSize: "0.8125rem" }}'),
      false,
    );
  }
  // The irreversibility sentence is owned by the block, not buried in the copy.
  assertEquals(commands.includes("record. This can't be undone."), false);
  assert(commands.includes('Your settings and workspace are kept.",'));
  assertEquals(commands.includes("The transcript stays on screen"), false);
});
