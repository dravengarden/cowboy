/** Pure rules of the browser chrome around the remote display: the address
 * field, which toolbar and menu controls apply in the current state, and the
 * binding of the attached viewer's key controls to the account, Machine and
 * instance the panel shows. Every command the chrome sends names exactly
 * that browser; nothing here reads the remote page. */
import {
  type BrowserScale,
  type BrowserStateEvent,
  type BrowserTarget,
  browserReasonText,
  browserStateLabel,
} from "./api";
import type { BrowserChord } from "./browserKeys";
import { type BrowserModel, browserRunning, sameTarget, shownTarget } from "./browserModel";
import { needsMachineChoice } from "./entryRules";

/** Key-level controls of one attached viewer. They act only on the session
 * of the viewer that published them, bound to `account` and `target`. */
export interface ViewerControls {
  account: string;
  target: BrowserTarget;
  /** This viewer's attachment, from its stream's first state. */
  attachmentId: string | undefined;
  chord(chord: BrowserChord): void;
  toggleKeyboard(): void;
  /** Set the remote clipboard to `text` and press Ctrl+V. */
  paste(text: string): void;
  /** Paste `url` into the address bar and press Return. */
  openAddress(url: string): void;
}

/** The viewer controls the chrome may use: only those of the viewer mounted
 * for this panel's account and exact Machine and instance. A late or foreign
 * viewer's controls are never used. */
export function controlsFor(
  controls: ViewerControls | undefined,
  account: string,
  model: BrowserModel,
): ViewerControls | undefined {
  const viewer = model.viewer?.target;
  if (!controls || !viewer || controls.account !== account) return undefined;
  if (controls.target.instance.account_id !== account) return undefined;
  return sameTarget(viewer, controls.target) ? controls : undefined;
}

export type ParsedAddress = { url: string } | { error: string };

const SCHEME = /^[a-z][a-z0-9+.-]*:/i;
const HOST_PORT = /^[^:/]+:\d+(?:[/?#]|$)/;

/** An owner-typed address as an `https` URL. A bare host gets `https://`;
 * any other scheme is refused, never rewritten. */
export function parseAddress(input: string): ParsedAddress {
  const text = input.trim();
  if (!text) return { error: "Enter an https address." };
  if (/\s/.test(text)) {
    return { error: "Enter a web address such as https://example.com; search is not available." };
  }
  const explicit = SCHEME.test(text) && !HOST_PORT.test(text);
  let url: URL;
  try {
    url = new URL(explicit ? text : `https://${text}`);
  } catch {
    return { error: "That is not a valid web address." };
  }
  if (url.protocol !== "https:") {
    return {
      error: `Only https addresses open in the Machine browser (this one is ${url.protocol.slice(0, -1)}).`,
    };
  }
  if (!explicit && !url.hostname.includes(".")) {
    return { error: "Enter a web address such as https://example.com." };
  }
  return { url: url.href };
}

export type AddressCommand =
  | { kind: "navigate"; url: string; target: BrowserTarget }
  | { kind: "error"; message: string };

/** What submitting the address field does: navigate exactly the running
 * browser this account's panel shows (the attached viewer's while one is
 * mounted), or say why not. */
export function addressCommand(
  model: BrowserModel,
  account: string,
  input: string,
): AddressCommand {
  const parsed = parseAddress(input);
  if ("error" in parsed) return { kind: "error", message: parsed.error };
  if (model.busy)
    return { kind: "error", message: "Wait for the current browser action to finish." };
  const target = shownTarget(model);
  if (
    !target ||
    target.instance.account_id !== account ||
    !browserRunning(model.read?.state?.state)
  ) {
    return { kind: "error", message: "Open the browser before entering an address." };
  }
  return { kind: "navigate", url: parsed.url, target };
}

export interface BrowserChromeState {
  /** Back, Forward and Reload: an attached, connected viewer. */
  navigation: boolean;
  keyboard: boolean;
  address: boolean;
  detach: boolean;
  hibernate: boolean;
  snapshot: boolean;
  /** Offered only when the running scale differs from this screen's. */
  restart: boolean;
  reset: boolean;
  paste: boolean;
  copy: boolean;
  refresh: boolean;
}

/** Which chrome controls apply now; `connected` is whether this panel's
 * viewer controls are bound (`controlsFor`). */
export function chromeState(
  model: BrowserModel,
  connected: boolean,
  preferredScale: BrowserScale,
): BrowserChromeState {
  const idle = !model.busy;
  const state = model.read?.state ?? null;
  const running = browserRunning(state?.state);
  // Deleting a retained profile stays the owner's while either gate is
  // closed; the Service refuses it only during revocation.
  const resettable =
    model.loaded &&
    !running &&
    state?.state !== "starting" &&
    (state !== null || (model.read?.placements.length ?? 0) > 0);
  return {
    navigation: connected,
    keyboard: connected,
    address: idle && running && shownTarget(model) !== undefined,
    detach: idle && model.viewer !== undefined,
    hibernate: idle && running,
    snapshot: idle && running,
    restart: idle && running && state?.scale !== undefined && state.scale !== preferredScale,
    reset: idle && resettable,
    paste: connected,
    copy: (model.clips?.items.length ?? 0) > 0,
    refresh: idle,
  };
}

const RETRYABLE = new Set(["memory", "capacity", "busy"]);

/** A refused open that frees up by itself (memory, capacity, another
 * operation) offers Retry, unless the owner must first choose a Machine. */
export function retryOffered(model: BrowserModel, pinned: boolean): boolean {
  const state = model.read?.state;
  if (!model.error || model.busy || model.viewer || !state?.reason) return false;
  if (!RETRYABLE.has(state.reason) || browserRunning(state.state)) return false;
  return pinned || !needsMachineChoice(model.read);
}

/** A remaining time for the owner: seconds while it is short, else whole
 * minutes. */
export function remainingText(ms: number): string {
  const seconds = Math.max(0, Math.round(ms / 1000));
  return seconds < 90 ? `${seconds} s` : `about ${Math.round(seconds / 60)} min`;
}

/** State line for the chrome and the empty page: label, reason and, for a
 * browser nobody views, when its grace ends. `deadline_ms` is when the
 * Machine stops the instance if nothing changes: the end of the detached
 * grace while detached, the graceful stop ahead of the runtime cap while
 * attached. */
export function browserStateSummary(state: BrowserStateEvent | null, now = Date.now()): string {
  const parts = [browserStateLabel(state?.state)];
  const reason = browserReasonText(state?.reason);
  if (reason) parts.push(reason);
  if (state?.state === "detached" && state.deadline_ms !== undefined) {
    parts.push(`Hibernates in ${remainingText(state.deadline_ms - now)} unless attached`);
  }
  if (state?.unsaved_state_lost) parts.push("The last stop lost unsaved page state");
  return parts.join(" · ");
}

/** How long before its runtime cap an attached viewer is told. */
export const CAP_WARNING_MS = 120_000;

/** When the Machine stops the browser an attached viewer shows (the
 * graceful stop ahead of its runtime cap), when the state names it. */
export function attachedStopTime(state: BrowserStateEvent | null): number | undefined {
  return state?.state === "attached" ? state.deadline_ms : undefined;
}

/** How long from `now` until the viewer is told; zero once it is that close. */
export function capWarningDelay(stopsAt: number, now: number): number {
  return Math.max(0, stopsAt - CAP_WARNING_MS - now);
}

/** The cap hibernates the browser with its snapshot, and the open panel
 * reopens it (a stopped browser ends the stream like any drop): sign-ins
 * stay, the open pages do not. */
export function capWarningText(stopsAt: number, now: number): string {
  return `This browser reaches its running-time limit in ${remainingText(stopsAt - now)}. It is then saved and restarted on a new tab: your sign-ins stay, open pages do not, so finish the step you are on.`;
}
