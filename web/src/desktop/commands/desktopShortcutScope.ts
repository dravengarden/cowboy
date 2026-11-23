import { desktopImeOwnsKey } from "./imeShortcut";
import { matchesShortcut, parseShortcut } from "./shortcut";

interface QueryRoot {
  querySelector(selectors: string): unknown;
}

const OVERLAY_SELECTOR =
  "[data-desktop-shortcut-scope='exclusive'], [role='dialog'][aria-modal='true'], .MuiPopover-root";

let cachedAppHost: Element | null = null;

function appHostChild(root: Document): Element | null {
  const byId = root.getElementById("root");
  if (byId) return byId;
  if (cachedAppHost?.isConnected && cachedAppHost.parentElement === root.body) {
    return cachedAppHost;
  }
  const pane = root.querySelector("[data-desktop-pane]");
  let node: Element | null = pane;
  while (node && node.parentElement !== root.body) node = node.parentElement;
  cachedAppHost = node;
  return node;
}

/**
 * The topmost Desktop overlay owns keyboard input before the workspace below.
 * Explicit scopes cover custom overlays; MUI Dialog/Popover roots make the
 * invariant automatic for ordinary product surfaces and menus.
 */
export function desktopOverlayOwnsShortcuts(root: QueryRoot): boolean {
  if (typeof Document !== "undefined" && root instanceof Document && root.body) {
    const active = root.activeElement;
    if (active instanceof Element && active.closest(OVERLAY_SELECTOR) !== null) {
      return true;
    }
    const appRoot = appHostChild(root);
    const children = root.body.children;
    for (let i = children.length - 1; i >= 0; i -= 1) {
      const child = children[i];
      if (!child || child === appRoot) continue;
      if (child.matches(OVERLAY_SELECTOR) || child.querySelector(OVERLAY_SELECTOR) !== null) {
        return true;
      }
    }
    return false;
  }
  return root.querySelector(OVERLAY_SELECTOR) !== null;
}

/** A printable CodeMirror input can bypass workspace navigation. Preserve any
 * direct command that explicitly opted into editor input. */
export function codeMirrorOwnsPrintableKey(
  event: KeyboardEvent,
  commands: Iterable<{
    shortcut?: string;
    allowInEditor?: boolean | ((target: EventTarget | null) => boolean);
  }>,
  mac: boolean,
): boolean {
  if (
    event.key.length !== 1 || event.metaKey || event.ctrlKey || event.altKey ||
    !(event.target instanceof Element && event.target.matches(".cm-content[contenteditable='true']"))
  ) return false;
  for (const { shortcut, allowInEditor } of commands) {
    if (shortcut && allowInEditor && matchesShortcut(parseShortcut(shortcut), event, mac)) {
      return false;
    }
  }
  return true;
}

/**
 * The Machine browser's remote viewer owns every key while it has focus:
 * Space, the workspace prefix, Esc and browser shortcuts all belong to the
 * remote page, so the Desktop router (leaders included) stands aside. Only
 * the viewer's own release chord is handled, by the browser modal.
 */
export function remoteViewerOwnsKey(event: Pick<KeyboardEvent, "target">): boolean {
  const target = event.target as { closest?: (selector: string) => unknown } | null;
  return typeof target?.closest === "function" &&
    target.closest("[data-machine-browser-viewer]") !== null;
}

/**
 * Surfaces below the Machine browser (Usage, Settings) stand aside while it
 * is open: the modal, its menus and its remote viewer own every key, so a
 * key typed into the remote page never switches or closes the page beneath.
 */
export function machineBrowserOwnsKeys(
  event: Pick<KeyboardEvent, "target">,
  root: QueryRoot,
): boolean {
  return remoteViewerOwnsKey(event) || root.querySelector("[data-machine-browser-modal]") !== null;
}

/**
 * The keyup of a Space consumed as the leader must not also activate a
 * focused button (Firefox activates buttons on Space keyup). A keyup inside
 * the remote viewer belongs to the remote page, which received its keydown:
 * it is never swallowed, so no Space stays held there.
 */
export function swallowLeaderSpaceKeyUp(
  armed: { current: boolean },
): (event: KeyboardEvent) => void {
  return (event) => {
    if (event.code !== "Space" || !armed.current) return;
    armed.current = false;
    if (remoteViewerOwnsKey(event)) return;
    event.preventDefault();
    event.stopImmediatePropagation();
  };
}

/** A key a page-level handler below the Machine browser leaves alone: an
 * IME owns it, or the Machine browser is open above the page. */
export function pageBelowBrowserSkipsKey(event: KeyboardEvent): boolean {
  return desktopImeOwnsKey(event) || machineBrowserOwnsKeys(event, document);
}
