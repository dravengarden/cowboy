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

/** Whether quick links are offered now: not while a command runs or the
 * browser is starting or stopping (a tap would do nothing). */
export function quickDisabled(model: BrowserModel): boolean {
  const state = model.read?.state?.state;
  return Boolean(model.busy) || state === "starting" || state === "stopping";
}

/** The browser identity the panel shows now. */
export function panelIdentity(model: BrowserModel): string | undefined {
  return model.observedIdentity ?? model.read?.browser_identity;
}

/** A quick link waiting for its browser: bound to the identity it was
 * tapped for, the error shown then, and whether only a confirmed takeover
 * may carry it. */
export interface PendingQuickLink {
  url: string;
  identity: string | undefined;
  takeover: boolean;
  error: string;
}

/** What a waiting quick link does now. It navigates only through this
 * panel's connected viewer (`connected`: the stream opened for exactly the
 * viewer's target), and is dropped when the browser identity changed, a
 * command or attachment failed since the tap, or the takeover it waits on
 * was cancelled. */
export function quickStep(
  pending: PendingQuickLink,
  now: {
    identity: string | undefined;
    connected: BrowserTarget | undefined;
    confirming: string | undefined;
    takeoverViewer: boolean;
    error: string;
  },
): "wait" | "navigate" | "drop" {
  if (
    pending.identity !== undefined &&
    now.identity !== undefined &&
    now.identity !== pending.identity
  ) {
    return "drop";
  }
  if (now.error && now.error !== pending.error) return "drop";
  if (pending.takeover && now.confirming !== "takeover" && !now.takeoverViewer) return "drop";
  return now.connected ? "navigate" : "wait";
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

/** What a quick-link tap does now: navigate the connected viewer, or wait
 * for it after opening (on the Machine the owner picks in the chooser, for a
 * first open), attaching or a takeover the owner confirms; nothing while a
 * transition runs. */
export function quickLaunchPlan(
  model: BrowserModel,
  connected: BrowserTarget | undefined,
): "navigate" | "open" | "choose" | "attach" | "takeover" | "none" {
  if (quickDisabled(model)) return "none";
  if (connected) return "navigate";
  const entry = quickEntry(model);
  if (entry === "open") return needsMachineChoice(model.read) ? "choose" : "open";
  return entry === "wait" ? "none" : entry;
}
