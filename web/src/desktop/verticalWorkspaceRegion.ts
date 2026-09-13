import type { DesktopPane } from "./DesktopWorkspaceController";

const PRIMARY_REGION: Record<DesktopPane, string> = {
  sessions: "sessions.list",
  prompt: "prompt.composer",
  conversation: "conversation.transcript",
};

/**
 * Resolve Ctrl-J/K as movement between workspace-level surfaces.
 *
 * Prompt plan, queue, and draft are auxiliary panels, not Vim windows. They
 * have dedicated commands and must never intercept movement between the active
 * pane and the top bar.
 */
export function verticalWorkspaceRegion(
  focusedPane: DesktopPane,
  focusedRegion: string | null,
  delta: -1 | 1,
): string | null {
  if (focusedRegion === "topbar.controls") {
    return delta === 1 ? PRIMARY_REGION[focusedPane] : null;
  }
  return delta === -1 ? "topbar.controls" : null;
}

/** The Top Bar is a workspace region, not a pane. While it owns keyboard
 * input, Sessions/Prompt/Conversation headers must not keep pane chrome. */
export function paneChromeOwnsFocus(
  focusedPane: DesktopPane,
  focusedRegion: string | null,
  pane: DesktopPane,
): boolean {
  if (focusedRegion === "topbar.controls") return false;
  return focusedPane === pane;
}
