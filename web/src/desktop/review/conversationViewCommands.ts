import type { DesktopCommand } from "../commands/DesktopCommandProvider";
import type { DesktopPane } from "../DesktopWorkspaceController";
import {
  desktopLeaderSequence,
  DESKTOP_WORKSPACE_KEYS,
} from "../commands/workspaceShortcuts";
import { desktopReviewShown, requestDesktopWorkView } from "./desktopWorkView";

/** `␣C`: focus the Conversation, returning from Review first. Leaving Review
 *  is always this deliberate key; `Esc` never does it (Review reads code, and
 *  Esc is pressed there constantly). */
export function focusConversationCommand(
  focusPane: (pane: DesktopPane) => void,
  when: () => boolean,
): DesktopCommand {
  return {
    id: "workspace.focusConversation",
    title: "Focus Conversation",
    description: "Show and focus the Conversation (from Review too)",
    group: "Workspace",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusConversation),
    when,
    run: () => {
      if (!desktopReviewShown()) {
        focusPane("conversation");
        return;
      }
      requestDesktopWorkView("conversation");
      // The transcript is shown after the view change commits.
      requestAnimationFrame(() => focusPane("conversation"));
    },
  };
}

/** Transcript-only bare keys (Follow, History/Explore, Page Index) do not
 *  answer while the work pane shows Review. */
export function transcriptShown(): boolean {
  return !desktopReviewShown();
}

export const TRANSCRIPT_HIDDEN_REASON = "Return to the Conversation first";
