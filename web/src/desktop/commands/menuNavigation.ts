import { desktopKeyIntent } from "./keyIntent";

/**
 * FOCUS.md "Menus": every Desktop menu and listbox popup speaks the Vim list
 * grammar. MUI keeps ownership of the ARIA behaviour (disabled rows, wrap,
 * typeahead, focus-visible); the Vim keys are re-dispatched as the native key
 * MUI already handles, so no menu needs a component-local key handler.
 *
 * - `j/k`: next/previous row (ArrowDown/ArrowUp)
 * - `gg/G`: first/last row (Home/End)
 * - `l`: activate (Enter)
 * - `h`: close (Escape)
 *
 * Letters resolve from physical codes so a CJK input source on menu chrome
 * still moves. A text field inside the popup (Autocomplete) owns its keys.
 */
const NATIVE: Readonly<Record<string, string>> = {
  j: "ArrowDown",
  k: "ArrowUp",
  G: "End",
  l: "Enter",
  h: "Escape",
};

/** How long `g` waits for its second `g`. */
const G_CHORD_MS = 1200;
let pendingG = 0;

export const DESKTOP_MENU_SELECTOR =
  "[role='menu'], [role='listbox'], [role='menubar']";

export function desktopMenuNavigationKey(
  event: KeyboardEvent,
  now = Date.now(),
): boolean {
  const target = event.target instanceof HTMLElement ? event.target : null;
  const menu = target?.closest<HTMLElement>(DESKTOP_MENU_SELECTOR);
  if (!target || !menu || event.shiftKey && event.code !== "KeyG") return false;
  const intent = desktopKeyIntent(event);
  if (intent.owner !== "command" || intent.modified) return false;
  const key = intent.key;
  if (key === "g") {
    event.preventDefault();
    event.stopPropagation();
    if (event.repeat) return true;
    if (pendingG && now - pendingG <= G_CHORD_MS) {
      pendingG = 0;
      redispatch(target, "Home");
    } else {
      pendingG = now;
    }
    return true;
  }
  pendingG = 0;
  const native = NATIVE[key];
  if (!native) return false;
  event.preventDefault();
  event.stopPropagation();
  // Holding j/k scrolls a long menu; a held l/h must not repeat an action.
  if (event.repeat && (key === "l" || key === "h")) return true;
  redispatch(target, native);
  return true;
}

function redispatch(target: HTMLElement, key: string): void {
  target.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      code: key,
      bubbles: true,
      cancelable: true,
    }),
  );
}
