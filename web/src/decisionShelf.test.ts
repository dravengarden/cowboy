import { assert, assertEquals } from "jsr:@std/assert";

/** Comment-free, whitespace-collapsed view: these are assertions about the
 *  STYLE, and `deno fmt` is free to rewrap any of these template literals. */
function code(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/^\s*\/\/.*$/gm, " ")
    .replace(/\s+/g, " ");
}

const shelfSource = await Deno.readTextFile(
  new URL("./decisionShelf.ts", import.meta.url),
);
const shelf = code(shelfSource);
const emphasis = shelf.slice(
  shelf.indexOf("export function decisionActionEmphasis"),
  shelf.indexOf("export function decisionShelfSurface"),
);
const surface = shelf.slice(
  shelf.indexOf("export function decisionShelfSurface"),
);
const actions = await Deno.readTextFile(
  new URL("./MobileDecisionActions.tsx", import.meta.url),
);
const card = await Deno.readTextFile(
  new URL("./ObsidianSheet.tsx", import.meta.url),
);
const filterSheets = await Promise.all(
  [
    "./UsageLogs.tsx",
    "./ProviderUsageActivityDetails.tsx",
    "./ObservabilityFilters.tsx",
  ].map((file) => Deno.readTextFile(new URL(file, import.meta.url))),
);

// The lift is painted, never composited. A wide upward `box-shadow` on this
// strip is the exact thing iOS WebKit turned into a large purple rectangle over
// the frosted cover sheet (`fix(mobile): flatten new session action footer`).
// Gradients have no shadow/backdrop-filter interaction, so they cannot regress
// that way — keep the plate itself shadow-free.
Deno.test("the decision plate lifts with a gradient, never an upward shadow", () => {
  assert(surface.includes('bottom: "100%"'));
  assert(surface.includes("linear-gradient(to top"));
  assertEquals(/boxShadow|box-shadow/.test(surface), false);
  // Fades end in a zero-alpha version of their OWN colour: the `transparent`
  // keyword fades through black in Safari.
  assert(surface.includes("alpha(accent, 0)"));
  assert(surface.includes("alpha(shade, 0)"));
  assertEquals(/linear-gradient\([^)]*transparent/.test(surface), false);
});

Deno.test("the accent is the theme's, stated on the hairline, riser and glow", () => {
  assert(shelf.includes("const accent = t.palette.primary.main;"));
  assert(/borderTop: `1px solid \$\{\s*alpha\(accent/.test(surface));
  assert(/boxShadow: `0 6px 16px -8px \$\{\s*alpha\(accent/.test(emphasis));
  // Press physics: the cap settles AND its glow collapses; either alone reads
  // as a colour change rather than contact.
  assert(emphasis.includes('transform: "translateY(1px)"'));
  // A disabled action must not advertise depth it cannot deliver.
  assert(emphasis.includes('"&.Mui-disabled": { boxShadow: "none"'));
  assert(emphasis.includes("prefers-reduced-motion"));
  // Doubled selectors: `actions` children come from foreign call sites
  // (ConfirmSheet's NetworkButton), so this must outrank MUI's own rules
  // regardless of stylesheet order.
  assert(emphasis.includes(".MuiButton-contained.MuiButton-contained"));
  assert(emphasis.includes(".MuiButton-text.MuiButton-text"));
});

Deno.test("every Cancel/confirm surface takes the same material", () => {
  // The two real decision FOOTERS get the whole plate.
  assert(actions.includes("...decisionShelfSurface(theme),"));
  assert(actions.includes("...decisionActionEmphasis(theme),"));
  assert(card.includes("...decisionShelfSurface(theme),"));
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
