/** Quick-launch pages of the Machine browser: the common sign-in pages an
 * owner opens first. Viewer-side only: a quick link is an ordinary owner
 * navigation of the running browser through the navigate command, so nothing
 * is injected into the browser and no page can tell it apart from typing
 * the address. See docs/machine-browser.md, "Viewer (core web)". */
import type { BrowserTarget } from "./api";
import { type BrowserModel, browserRunning } from "./browserModel";

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

/** The browser a quick link navigates now: the one this panel views,
 * running and idle. Otherwise the link waits until the panel views it. */
export function quickTarget(model: BrowserModel): BrowserTarget | undefined {
  if (model.busy || !model.viewer) return undefined;
  return browserRunning(model.read?.state?.state) ? model.viewer.target : undefined;
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
