/** Quick-launch pages of the Machine browser: the common sign-in pages an
 * owner opens first. Viewer-side only: a quick link is an ordinary owner
 * navigation of the running browser through the navigate command, so nothing
 * is injected into the browser and no page can tell it apart from typing
 * the address. See docs/machine-browser.md, "Viewer (core web)". */
import type { BrowserTarget } from "./api";
import type { BrowserModel } from "./browserModel";
import { needsMachineChoice } from "./entryRules";

export interface QuickLink {
  key: string;
  label: string;
  url: string;
  /** The account-usage family the page signs in to, if any. */
  family?: string;
  /** Shown more prominently: the card's own page, and Google for a shared
   * browser (its members sign in with Google). */
  emphasized: boolean;
}

const PAGES: readonly Omit<QuickLink, "emphasized">[] = [
  { key: "anthropic", label: "Claude", url: "https://claude.ai/", family: "anthropic" },
  { key: "openai", label: "ChatGPT · Codex", url: "https://chatgpt.com/", family: "openai" },
  { key: "xai", label: "Grok", url: "https://grok.com/", family: "xai" },
  { key: "deepseek", label: "DeepSeek", url: "https://chat.deepseek.com/", family: "deepseek" },
  { key: "google", label: "Google sign-in", url: "https://accounts.google.com/" },
];

/** The quick links of a card's browser: the card's own family first (for a
 * shared browser, the family of the card that opened it), the other
 * services in their usual order, Google last; a shared browser emphasizes
 * Google too. */
export function quickLinks(family: string | undefined, shared: boolean): QuickLink[] {
  const own = PAGES.filter((page) => page.family !== undefined && page.family === family);
  const others = PAGES.filter((page) => page.family !== undefined && page.family !== family);
  const google = PAGES.filter((page) => page.family === undefined);
  return [...own, ...others, ...google].map((page) => ({
    ...page,
    emphasized: (page.family !== undefined && page.family === family) || (shared && !page.family),
  }));
}

/** Whether quick links are offered now: only once the initial browser
 * read answered (admission, Machine choice and, against a Service with
 * shared profiles, the browser identity known), and not while a command
 * runs or the browser starts or stops (a tap would do nothing). */
export function quickDisabled(model: BrowserModel, identityRequired = false): boolean {
  const read = model.read;
  if (!read || read.admission) return true;
  if (identityRequired && panelIdentity(model) === undefined) return true;
  const state = read.state?.state;
  return Boolean(model.busy) || state === "starting" || state === "stopping";
}

/** The browser identity the panel shows now. */
export function panelIdentity(model: BrowserModel): string | undefined {
  return model.observedIdentity ?? model.read?.browser_identity;
}

/** The viewer a pending link is bound to: one attachment, by the lifecycle
 * epoch it was established in, its generation and its exact target. */
export interface QuickViewer {
  since: number;
  generation: number;
  target: BrowserTarget;
}

export function viewerOf(model: BrowserModel): QuickViewer | undefined {
  const viewer = model.viewer;
  return (
    viewer && { since: viewer.since ?? 0, generation: viewer.generation, target: viewer.target }
  );
}

function sameViewer(left: QuickViewer, right: QuickViewer | undefined): boolean {
  return (
    right !== undefined &&
    left.since === right.since &&
    left.generation === right.generation &&
    sameBrowserTarget(left.target, right.target)
  );
}

function sameBrowserTarget(left: BrowserTarget, right: BrowserTarget | undefined): boolean {
  return (
    right !== undefined &&
    left.machine_id === right.machine_id &&
    left.instance.instance_id === right.instance.instance_id &&
    left.instance.incarnation === right.instance.incarnation
  );
}

/** A quick link waiting for its browser. It is bound to the browser
 * identity it was tapped for, the lifecycle epoch then (only a viewer
 * established after it, or the viewer already connecting, may carry it), the
 * Machine it opens on when known, the error shown then, and whether only a
 * confirmed takeover may carry it. Once a viewer appears the link binds to
 * exactly that attachment. */
export interface PendingQuickLink {
  url: string;
  identity: string | undefined;
  takeover: boolean;
  error: string;
  sinceEpoch: number;
  machine?: string | undefined;
  viewer?: QuickViewer | undefined;
}

export interface QuickNow {
  /** The panel's identity now, and the sheet's (inventory) binding. */
  identity: string | undefined;
  sheetIdentity: string | undefined;
  viewer: QuickViewer | undefined;
  /** This panel's connected viewer's target (the stream is open). */
  connected: BrowserTarget | undefined;
  confirming: string | undefined;
  takeoverViewer: boolean;
  error: string;
}

/** The next state of a waiting quick link: keep waiting (possibly newly
 * bound to the viewer that appeared), navigate its bound and connected
 * viewer, or drop it: another browser identity (from a read, an answer or
 * the inventory), a failed command or attachment, its attachment ended or
 * replaced, a viewer on another Machine, or a cancelled takeover. */
export function quickStep(
  pending: PendingQuickLink,
  now: QuickNow,
): { action: "wait"; pending: PendingQuickLink } | { action: "navigate" | "drop" } {
  const moved = (identity: string | undefined): boolean =>
    pending.identity !== undefined && identity !== undefined && identity !== pending.identity;
  if (moved(now.identity) || moved(now.sheetIdentity)) return { action: "drop" };
  if (now.error && now.error !== pending.error) return { action: "drop" };
  let bound = pending;
  if (!pending.viewer) {
    if (pending.takeover && now.confirming !== "takeover" && !now.takeoverViewer) {
      return { action: "drop" };
    }
    const viewer = now.viewer;
    if (!viewer || viewer.since <= pending.sinceEpoch) return { action: "wait", pending };
    if (pending.machine !== undefined && viewer.target.machine_id !== pending.machine) {
      return { action: "drop" };
    }
    bound = { ...pending, viewer };
  } else if (!sameViewer(pending.viewer, now.viewer)) {
    return { action: "drop" };
  }
  return now.connected && sameBrowserTarget(bound.viewer!.target, now.connected)
    ? { action: "navigate" }
    : { action: "wait", pending: bound };
}

/** What a quick link does first when the panel does not view a running
 * browser: open a stopped one, attach to one running unviewed, or ask to
 * take over one another device views. Starting or stopping: wait. */
export function quickEntry(model: BrowserModel): "open" | "attach" | "takeover" | "wait" {
  const state = model.read?.state?.state;
  if (model.busy || state === "starting" || state === "stopping") return "wait";
  if (state === "detached") return "attach";
  if (state === "attached") return model.viewer ? "wait" : "takeover";
  return "open";
}

/** What a quick-link tap does now: navigate the connected viewer; wait for
 * the attachment already connecting (never a second attach); or open (on
 * the Machine the owner picks in the chooser, for a first open), attach or
 * take over (once confirmed) and then wait; nothing while disabled. */
export function quickLaunchPlan(
  model: BrowserModel,
  connected: BrowserTarget | undefined,
  identityRequired = false,
): "navigate" | "reuse" | "open" | "choose" | "attach" | "takeover" | "none" {
  if (quickDisabled(model, identityRequired)) return "none";
  if (connected) return "navigate";
  if (model.viewer || model.awaitingOpen) return "reuse";
  const entry = quickEntry(model);
  if (entry === "open") return needsMachineChoice(model.read) ? "choose" : "open";
  return entry === "wait" ? "none" : entry;
}
