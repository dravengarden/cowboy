// Last-screen snapshot for an instant, progressive open
// (docs/offline-first-sync.md §Boot presentation).
//
// The app is client-rendered and per-user, so it cannot be pre-rendered at
// build time. Instead it pre-renders ITSELF: when the screen is at rest it
// saves a sanitized static copy of what `#root` shows, plus the CSS that copy
// uses. On the next open `index.html` mounts that copy in a closed shadow root
// above the booting app and cross-fades it away once the live app has painted.
// The user sees their real last screen in the first frames, and live content
// replaces it in place.
//
// This module has no store or React dependency: the auth layer clears the
// snapshot and must never open the product socket.

export const BOOT_SNAPSHOT_CACHE = "cowboy-boot-snapshot-v1";
export const BOOT_SNAPSHOT_URL = "/__boot/snapshot.json";
/** What the document can learn about the saved screen SYNCHRONOUSLY, before
 *  Cache Storage answers: enough to decide whether one is coming, so the
 *  skeleton is not painted only to be replaced a moment later. */
export const BOOT_SNAPSHOT_HINT_KEY = "cowboy:boot-snapshot";
export const BOOT_THEME_KEY = "cowboy:boot-theme";
export const BOOT_SURFACE_KEY = "cowboy:boot-surface";
/** Parsing a snapshot larger than this costs more than the wait it saves. */
export const BOOT_SNAPSHOT_MAX_CHARS = 1_000_000;
const INLINE_IMAGE_MAX_CHARS = 65_536;
/** The overlay cannot scroll, so only the viewport is worth keeping. The
 * margin leaves room for shadows and just-off-screen sticky chrome. */
const VIEWPORT_MARGIN_PX = 64;

export interface BootSnapshot {
  readonly v: 1;
  readonly user: string;
  readonly sessionId: string | null;
  readonly width: number;
  readonly height: number;
  readonly scheme: "light" | "dark";
  /** `system` follows the OS scheme, so a changed scheme invalidates it. */
  readonly themeMode: string;
  readonly savedAt: number;
  readonly htmlClass: string;
  readonly htmlStyle: readonly (readonly [string, string])[];
  readonly css: string;
  /** `@font-face` is ignored inside a shadow tree; it goes to the document. */
  readonly fontCss: string;
  readonly html: string;
}

export interface BootSnapshotContext {
  readonly user: string;
  readonly sessionId: string | null;
  readonly themeMode: string;
  /** A streaming turn repaints constantly, so the periodic capture skips it.
   * Leaving the app still saves the screen the user actually last saw. */
  readonly busy: boolean;
}

// --- Pure policy (unit-tested) ----------------------------------------------

/** Split a selector list on its top-level commas. */
export function splitSelectorList(selector: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let quote = "";
  let start = 0;
  for (let index = 0; index < selector.length; index += 1) {
    const char = selector[index]!;
    if (quote !== "") {
      if (char === "\\") index += 1;
      else if (char === quote) quote = "";
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "(" || char === "[") depth += 1;
    else if (char === ")" || char === "]") depth -= 1;
    else if (char === "," && depth === 0) {
      parts.push(selector.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(selector.slice(start).trim());
  return parts.filter((part) => part !== "");
}

/** Class names a selector mentions. */
export function classTokens(selector: string): string[] {
  return [...selector.matchAll(/\.(-?[_a-zA-Z][\w-]*)/g)].map((match) => match[1]!);
}

/** Whether a style rule can still match something in the snapshot. One
 * alternative of a selector list matches only if EVERY class it names is
 * present — that is what prunes the bulk of an app-wide sheet. A functional
 * pseudo-class (`:not(.a)`, `:is(.a, .b)`) breaks that implication, so there
 * any one present class keeps the rule. A classless rule is global. */
export function keepStyleRule(selector: string, used: ReadonlySet<string>): boolean {
  return splitSelectorList(selector).some((alternative) => {
    const tokens = classTokens(alternative);
    if (tokens.length === 0) return true;
    return /:(is|where|not|has|matches|any)\(/i.test(alternative)
      ? tokens.some((token) => used.has(token))
      : tokens.every((token) => used.has(token));
  });
}

/** A shadow tree contains no `html` or `body`, so the rules that carry the
 * document's custom properties and base typography are retargeted at the
 * host. Children inherit from it exactly as they did from `<html>`. */
export function scopedSelector(selector: string): string {
  return selector
    .replace(/(^|[\s>+~])(?::root|html|body)(?![\w-])/g, "$1:host")
    .replace(/:host\s*[\s>]\s*:host(?![\w-])/g, ":host");
}

/** The `<html>` inline declarations a snapshot may carry: the global font
 * scale (every `rem` depends on it), the canvas colour, and Cowboy's own
 * custom properties. Mirrors the allow-list in `index.html`. */
export function restorableHtmlStyle(name: string): boolean {
  return /^(font-size|background-color|--(cowboy|vv|kb)-[\w-]+)$/.test(name);
}

/** Attributes that must not survive into a static copy. `id` stays: the shadow
 * root scopes it away from the document, and SVG paint servers (`url(#id)`)
 * stop rendering without it. */
export function strippedAttribute(name: string, value: string): boolean {
  const lower = name.toLowerCase();
  return lower.startsWith("on") || lower === "tabindex" || lower === "autofocus" ||
    lower === "contenteditable" || lower === "srcdoc" || /^\s*javascript:/i.test(value);
}

/** Whether a box lies wholly outside the viewport the overlay will cover. */
export function outsideViewport(
  box: { left: number; top: number; right: number; bottom: number },
  width: number,
  height: number,
  margin = VIEWPORT_MARGIN_PX,
): boolean {
  return box.right < -margin || box.bottom < -margin ||
    box.left > width + margin || box.top > height + margin;
}

// --- Capture (browser only) --------------------------------------------------

const REMOVED_NODES = "script, style, link, meta, base, noscript, template, " +
  "[data-mobile-sync-pill], [data-boot-exclude]";
/** Live or scripted content: keep the space it occupied, drop the element. */
const BOXED_NODES = new Set(["IFRAME", "OBJECT", "EMBED", "CANVAS", "VIDEO", "AUDIO"]);

function restingScreen(root: HTMLElement): boolean {
  // Sheets, dialogs and menus portal to <body>; their presence means open.
  if (
    document.querySelector(
      "[data-detent-sheet='true'], [data-obsidian-sheet], .MuiModal-root, .MuiPopover-root, " +
        "[role='dialog'][aria-modal='true'], [data-mobile-pager-modal='true']",
    )
  ) return false;
  // A boot, setup or loading screen; a drawer; a surface mid-gesture; the
  // keyboard; or the Review page, whose content is the workspace read over the
  // network (the boot script never shows a snapshot for it either).
  if (
    root.querySelector(
      "#app-splash, [data-boot-loading], [data-machine-setup-gate], " +
        "[data-mobile-drawer-open='true'], [data-mobile-drawer-moving='true'], " +
        "[data-mobile-product-moving='true'], [data-mobile-keyboard-open='true'], " +
        "[data-mobile-product='review']",
    )
  ) return false;
  const focused = document.activeElement;
  return !(focused instanceof HTMLElement &&
    (focused.isContentEditable || focused.tagName === "TEXTAREA" || focused.tagName === "INPUT"));
}

function pinBox(copy: HTMLElement, box: DOMRect): void {
  copy.style.setProperty("box-sizing", "border-box");
  copy.style.setProperty("width", `${String(Math.round(box.width))}px`);
  copy.style.setProperty("height", `${String(Math.round(box.height))}px`);
  copy.style.setProperty("flex-shrink", "0");
}

/** Walk the live tree and its clone in step — they are still identical — and
 * turn the clone into a static picture of what the viewport shows. */
function freeze(original: Element, copy: Element, width: number, height: number): void {
  if (!(original instanceof HTMLElement) || !(copy instanceof HTMLElement)) return;
  const box = original.getBoundingClientRect();
  if (BOXED_NODES.has(original.tagName)) {
    const placeholder = document.createElement("div");
    placeholder.className = copy.className;
    placeholder.style.cssText = copy.style.cssText;
    placeholder.style.setProperty("display", "inline-block");
    pinBox(placeholder, box);
    copy.replaceWith(placeholder);
    return;
  }
  // `display: contents` and zero-height wrappers report an empty box while
  // their children are visible, so only a real box may be dropped.
  if ((box.width > 0 || box.height > 0) && outsideViewport(box, width, height)) {
    // Off-screen: keep the space (scroll offsets are measured from it) and
    // throw the content away. This is what keeps a long transcript small.
    copy.replaceChildren();
    pinBox(copy, box);
    return;
  }
  if (original.scrollTop !== 0 || original.scrollLeft !== 0) {
    copy.setAttribute(
      "data-boot-scroll",
      `${String(Math.round(original.scrollTop))},${String(Math.round(original.scrollLeft))}`,
    );
  }
  if (original instanceof HTMLTextAreaElement) {
    copy.textContent = original.value;
  } else if (original instanceof HTMLInputElement) {
    if (original.type === "password") copy.removeAttribute("value");
    else copy.setAttribute("value", original.value);
  } else if (original instanceof HTMLImageElement) {
    // Pin the box so a slow or missing image cannot reflow the picture.
    pinBox(copy, box);
    if (original.src.startsWith("data:") && original.src.length > INLINE_IMAGE_MAX_CHARS) {
      copy.removeAttribute("src");
      copy.removeAttribute("srcset");
    }
  }
  // Pair the children up first: replacing a boxed child mutates the clone's
  // collection while the walk is reading it.
  const pairs: [Element, Element][] = [];
  for (let index = 0; index < original.children.length; index += 1) {
    const child = copy.children[index];
    if (child !== undefined) pairs.push([original.children[index]!, child]);
  }
  for (const [from, to] of pairs) freeze(from, to, width, height);
}

function staticCopy(root: HTMLElement): HTMLElement {
  const clone = root.cloneNode(true) as HTMLElement;
  freeze(root, clone, globalThis.innerWidth, globalThis.innerHeight);
  // The overlay owns the root box; never carry a pinned size on it.
  clone.removeAttribute("style");
  for (const node of clone.querySelectorAll(REMOVED_NODES)) node.remove();
  for (const node of clone.querySelectorAll("*")) {
    for (const attribute of [...node.attributes]) {
      if (strippedAttribute(attribute.name, attribute.value)) node.removeAttribute(attribute.name);
    }
  }
  return clone;
}

function ruleHeader(rule: CSSRule): string {
  const text = rule.cssText;
  const brace = text.indexOf("{");
  return brace === -1 ? text : text.slice(0, brace).trim();
}

function serializeRules(
  rules: CSSRuleList,
  used: ReadonlySet<string>,
  css: string[],
  fonts: string[],
): void {
  for (const rule of rules) {
    if (rule instanceof CSSFontFaceRule) {
      fonts.push(rule.cssText);
    } else if (rule instanceof CSSStyleRule) {
      if (!keepStyleRule(rule.selectorText, used)) continue;
      const selector = scopedSelector(rule.selectorText);
      css.push(
        selector === rule.selectorText ? rule.cssText : `${selector} { ${rule.style.cssText} }`,
      );
    } else if (rule instanceof CSSKeyframesRule) {
      css.push(rule.cssText);
    } else if (rule instanceof CSSImportRule) {
      try {
        if (rule.styleSheet) serializeRules(rule.styleSheet.cssRules, used, css, fonts);
      } catch {
        // A cross-origin import cannot be read.
      }
    } else if ("cssRules" in rule && rule.cssRules instanceof CSSRuleList) {
      // @media, @supports, @container, @layer blocks.
      const inner: string[] = [];
      serializeRules(rule.cssRules, used, inner, fonts);
      if (inner.length > 0) css.push(`${ruleHeader(rule)} { ${inner.join("\n")} }`);
    }
  }
}

function usedStyles(clone: HTMLElement): { css: string; fontCss: string } {
  // The document's own classes count: their rules become `:host` rules, which
  // is where the snapshot inherits the font scale and palette from.
  const used = new Set<string>([
    ...document.documentElement.classList,
    ...document.body.classList,
  ]);
  for (const node of clone.querySelectorAll("*")) {
    for (const name of node.classList) used.add(name);
  }
  const css: string[] = [];
  const fonts: string[] = [];
  // Vite serves emotion's sheet in speedy mode: the <style> has no text, only
  // live `cssRules`. Reading the rules is therefore the only way to get it.
  const sheets: CSSStyleSheet[] = [...document.styleSheets, ...document.adoptedStyleSheets];
  for (const sheet of sheets) {
    try {
      serializeRules(sheet.cssRules, used, css, fonts);
    } catch {
      // A cross-origin sheet cannot be read; the snapshot simply lacks it.
    }
  }
  return { css: css.join("\n"), fontCss: fonts.join("\n") };
}

/** Save the current resting screen. Resolves `false` when there is nothing
 * worth saving; never throws. */
export async function captureBootSnapshot(context: BootSnapshotContext): Promise<boolean> {
  try {
    const root = document.getElementById("root");
    if (!root || typeof caches === "undefined" || !restingScreen(root)) return false;
    const clone = staticCopy(root);
    const html = clone.innerHTML;
    const { css, fontCss } = usedStyles(clone);
    if (html.length + css.length + fontCss.length > BOOT_SNAPSHOT_MAX_CHARS) return false;
    const style = document.documentElement.style;
    const htmlStyle: [string, string][] = [];
    for (let index = 0; index < style.length; index += 1) {
      const name = style.item(index);
      if (restorableHtmlStyle(name)) htmlStyle.push([name, style.getPropertyValue(name).trim()]);
    }
    const snapshot: BootSnapshot = {
      v: 1,
      user: context.user,
      sessionId: context.sessionId,
      width: globalThis.innerWidth,
      height: globalThis.innerHeight,
      scheme: globalThis.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
      themeMode: context.themeMode,
      savedAt: Date.now(),
      htmlClass: document.documentElement.className,
      htmlStyle,
      css,
      fontCss,
      html,
    };
    const cache = await caches.open(BOOT_SNAPSHOT_CACHE);
    await cache.put(
      BOOT_SNAPSHOT_URL,
      new Response(JSON.stringify(snapshot), { headers: { "content-type": "application/json" } }),
    );
    try {
      const { css: _css, fontCss: _fontCss, html: _html, ...hint } = snapshot;
      globalThis.localStorage?.setItem(BOOT_SNAPSHOT_HINT_KEY, JSON.stringify(hint));
    } catch {
      // Without the hint the next open simply shows the skeleton first.
    }
    return true;
  } catch {
    return false;
  }
}

/** Forget the saved screen: sign-out, a login answer, another account. */
export function clearBootSnapshot(): void {
  try {
    globalThis.localStorage?.removeItem(BOOT_SNAPSHOT_HINT_KEY);
    if (typeof caches !== "undefined") {
      void caches.delete(BOOT_SNAPSHOT_CACHE).catch(() => undefined);
    }
  } catch {
    // No Cache Storage (native WKWebView): nothing was saved either.
  }
}

interface BootHandle {
  done: boolean;
  ready(): void;
}

/** The live app has painted what the user came for: let the overlay go. Two
 * frames later, so that paint is on screen before the cross-fade starts. */
export function signalBootReady(): void {
  const boot = (globalThis as { __cowboyBoot?: BootHandle }).__cowboyBoot;
  if (boot === undefined || boot.done) return;
  const frame = globalThis.requestAnimationFrame ?? ((run: () => void) => setTimeout(run, 16));
  frame(() => frame(() => boot.ready()));
}

/** Record which chrome the app actually chose, so the next open's skeleton
 * wears the same one. The app's rule reads `navigator.maxTouchPoints` and the
 * native host, neither of which CSS can see; the static shell approximates it
 * with media queries only until this has been written once. */
export function rememberBootSurface(desktop: boolean): void {
  try {
    globalThis.localStorage?.setItem(BOOT_SURFACE_KEY, desktop ? "desktop" : "touch");
  } catch {
    // Privacy mode: the shell keeps its media-query approximation.
  }
}

/** Record the app's real canvas colours for the static boot shell, so the
 * first frame of the NEXT open already matches the chosen appearance. */
export function rememberBootTheme(
  colours: { bg: string; ink: string; paper: string; line: string },
): void {
  try {
    globalThis.localStorage?.setItem(BOOT_THEME_KEY, JSON.stringify(colours));
  } catch {
    // Privacy mode: the shell falls back to the OS scheme.
  }
}

/** Backstop for a screen that changes without the app noticing. */
const PERIODIC_CAPTURE_MS = 60_000;

/** Keep the saved screen current WHILE THE APP IS ALIVE.
 *
 * The obvious trigger — save the screen as the user leaves — does not work:
 * writing to Cache Storage is asynchronous and a document being discarded
 * does not stay alive to finish it, so the write is simply lost. `pagehide`
 * and a backgrounding `visibilitychange` are therefore best-effort extras.
 * What the next open actually depends on is the capture that already
 * happened, a few seconds after the screen last settled. */
export function installBootSnapshot(context: () => BootSnapshotContext | null): () => void {
  const capture = (): void => {
    const current = context();
    if (current !== null && !current.busy) void captureBootSnapshot(current);
  };
  const onHidden = (): void => {
    if (document.visibilityState === "hidden") capture();
  };
  const onPeriodic = (): void => {
    if (document.visibilityState === "visible") capture();
  };
  document.addEventListener("visibilitychange", onHidden);
  globalThis.addEventListener("pagehide", capture);
  const timer = setInterval(onPeriodic, PERIODIC_CAPTURE_MS);
  return () => {
    document.removeEventListener("visibilitychange", onHidden);
    globalThis.removeEventListener("pagehide", capture);
    clearInterval(timer);
  };
}
