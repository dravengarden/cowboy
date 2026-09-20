// The boot presentation's pure policy, plus the contract that index.html's
// inline loader and this module still agree (docs/offline-first-sync.md
// §Boot presentation). The capture itself needs a live document and is
// verified in the browser.
import { assert, assertEquals, assertFalse } from "jsr:@std/assert";
import {
  BOOT_SNAPSHOT_CACHE,
  BOOT_SNAPSHOT_HINT_KEY,
  BOOT_SNAPSHOT_URL,
  BOOT_SURFACE_KEY,
  BOOT_THEME_KEY,
  classTokens,
  keepStyleRule,
  outsideViewport,
  restorableHtmlStyle,
  scopedSelector,
  splitSelectorList,
  strippedAttribute,
} from "./bootSnapshot.ts";

Deno.test("a selector list splits only on its top-level commas", () => {
  assertEquals(splitSelectorList(".a, .b > .c"), [".a", ".b > .c"]);
  assertEquals(splitSelectorList(":is(.a, .b) .c"), [":is(.a, .b) .c"]);
  assertEquals(splitSelectorList("[title='a,b'], .c"), ["[title='a,b']", ".c"]);
});

Deno.test("classTokens reads every class a selector names", () => {
  assertEquals(classTokens(".css-1ab .css-2cd:hover"), ["css-1ab", "css-2cd"]);
  assertEquals(classTokens("div[data-x='.y']"), ["y"]); // harmless over-read
  assertEquals(classTokens("html"), []);
});

Deno.test("a rule survives only when some alternative can still match", () => {
  const used = new Set(["css-a", "css-b"]);
  // Global rules always survive: they carry the reset and the type styles.
  assert(keepStyleRule("html, body", used));
  assert(keepStyleRule("*, *::before", used));
  assert(keepStyleRule(".css-a", used));
  // Every class of a plain compound must be present, or it cannot match.
  assertFalse(keepStyleRule(".css-a.css-missing", used));
  assertFalse(keepStyleRule(".css-missing .css-a", used));
  // One matching alternative is enough.
  assert(keepStyleRule(".css-missing, .css-b", used));
  // A functional pseudo-class breaks the implication, so any hit keeps it.
  assert(keepStyleRule(".css-a:not(.css-missing)", used));
  assertFalse(keepStyleRule(".css-missing:not(.css-other)", used));
});

Deno.test("document-level selectors are retargeted at the shadow host", () => {
  assertEquals(scopedSelector(":root"), ":host");
  assertEquals(scopedSelector("html, body"), ":host, :host");
  assertEquals(scopedSelector("body .css-a"), ":host .css-a");
  assertEquals(scopedSelector("html body .css-a"), ":host .css-a");
  // Never rewrite a class, attribute value or custom element that merely
  // contains the word.
  assertEquals(scopedSelector(".bodybuilder"), ".bodybuilder");
  assertEquals(scopedSelector("[data-x='body']"), "[data-x='body']");
  assertEquals(scopedSelector(".css-a"), ".css-a");
});

Deno.test("only presentation-critical html declarations are restorable", () => {
  // Every `rem` in the snapshot depends on the global font scale.
  assert(restorableHtmlStyle("font-size"));
  assert(restorableHtmlStyle("background-color"));
  assert(restorableHtmlStyle("--cowboy-font-scale"));
  assert(restorableHtmlStyle("--vv-height"));
  assert(restorableHtmlStyle("--kb-inset"));
  assertFalse(restorableHtmlStyle("position"));
  assertFalse(restorableHtmlStyle("--other-app"));
});

Deno.test("a static copy carries no script surface", () => {
  assert(strippedAttribute("onclick", "run()"));
  assert(strippedAttribute("ONCLICK", "run()"));
  assert(strippedAttribute("href", " javascript:run()"));
  assert(strippedAttribute("autofocus", ""));
  assert(strippedAttribute("contenteditable", "true"));
  // `id` stays: the shadow root scopes it, and SVG `url(#id)` paint servers
  // stop rendering without it.
  assertFalse(strippedAttribute("id", "gradient-1"));
  assertFalse(strippedAttribute("class", "css-a"));
  assertFalse(strippedAttribute("href", "/sessions/1"));
});

Deno.test("only boxes wholly off the viewport are dropped", () => {
  const box = (top: number, bottom: number) => ({ left: 0, right: 390, top, bottom });
  assertFalse(outsideViewport(box(0, 100), 390, 844));
  assertFalse(outsideViewport(box(800, 900), 390, 844)); // straddles the fold
  assertFalse(outsideViewport(box(-40, -10), 390, 844, 64)); // inside the margin
  assert(outsideViewport(box(-900, -800), 390, 844));
  assert(outsideViewport(box(2000, 2100), 390, 844));
});

Deno.test("index.html's inline loader and this module agree", async () => {
  const html = await Deno.readTextFile(new URL("../index.html", import.meta.url));
  assert(html.includes(JSON.stringify(BOOT_SNAPSHOT_CACHE)), BOOT_SNAPSHOT_CACHE);
  assert(html.includes(JSON.stringify(BOOT_SNAPSHOT_URL)), BOOT_SNAPSHOT_URL);
  assert(html.includes(JSON.stringify(BOOT_THEME_KEY)), BOOT_THEME_KEY);
  assert(html.includes(JSON.stringify(BOOT_SURFACE_KEY)), BOOT_SURFACE_KEY);
  assert(html.includes(JSON.stringify(BOOT_SNAPSHOT_HINT_KEY)), BOOT_SNAPSHOT_HINT_KEY);
  // The document must never be left on a bare canvas: every path that
  // declines the saved screen has to bring the skeleton back.
  assert(html.includes("boot-restoring"), "boot-restoring class");
  // One predicate, read synchronously before the first paint and again when
  // Cache Storage answers. Two copies drifted apart once already: the
  // document held the skeleton back for a snapshot the loader then refused.
  assertEquals(html.split("__cowboyBootEligible").length - 1, 3, "shared eligibility predicate");
  // Whatever the loader decides, it must never end on a bare canvas.
  assert(html.includes("return reveal()"), "declining the saved screen reveals the skeleton");
  assert(html.includes(".catch(reveal)"), "a failed lookup reveals the skeleton");
  // The skeleton's chrome follows the app's own last answer, not a width.
  assert(html.includes("html.boot-desktop"), "boot-desktop class");
  assert(html.includes("html.boot-touch"), "boot-touch class");
  // The overlay must stay inert, unfocusable and out of the accessibility
  // tree: it is a picture, not the app.
  assert(html.includes("host.inert = true"));
  assert(html.includes('attachShadow({ mode: "closed" })'));
  // A stuck app must never hide behind a picture of itself.
  assert(/setTimeout\(\(\) => boot\.ready\(\), \d+\)/.test(html));
  // The same declarations restorableHtmlStyle captures, split by where they
  // may be applied: only `font-size` has to reach <html>, because `rem`
  // resolves against the document root. Putting the captured viewport
  // variables there too would lay the booting app out against stale values.
  assert(html.includes(`name === "font-size" || name === "background-color"`));
  assert(html.includes("^--(cowboy|vv|kb)-[\\w-]+$"));
  assert(html.includes("host.style.setProperty(name, value)"));
  // The saved screen must not be shown in a face that is about to change:
  // the same text in a fallback has different metrics, so revealing it early
  // makes the whole screen reflow when the app repaints it.
  assert(html.includes("opacity:0"), "the overlay mounts rendered but invisible");
  // By the text this screen contains, not by family alone: a bare family name
  // only pulls the default unicode subset, so a Chinese transcript would be
  // drawn in a fallback and reflow when the real subset arrived.
  assert(html.includes("document.fonts.check(spec, sample)"), "checked against its own text");
  assert(html.includes("document.fonts.load(spec, sample)"), "and loaded with it");
  assert(/BOOT_FONT_WAIT_MS = \d+/.test(html), "with a bounded wait");
  assert(/BOOT_FONT_WAIT_MS = \d+/.test(html), "with a bounded wait");
});

Deno.test("the static boot shell and BootSkeleton render the same markup", async () => {
  const html = await Deno.readTextFile(new URL("../index.html", import.meta.url));
  const skeleton = await Deno.readTextFile(new URL("./BootSkeleton.tsx", import.meta.url));
  // Both must paint one shape, or the document's first frame would jump when
  // React takes over.
  for (const name of ["boot-shell", "boot-rail", "boot-main", "boot-feed", "boot-card", "boot-own", "boot-composer", "boot-tools", "boot-nav", "boot-gap", "boot-dot", "boot-bar"]) {
    assert(html.includes(name), `index.html is missing .${name}`);
    assert(skeleton.includes(name), `BootSkeleton is missing .${name}`);
  }
  const count = (source: string, token: string): number =>
    source.split(token).length - 1;
  assertEquals(count(html, 'class="boot-card'), count(skeleton, 'className="boot-card'));
});
