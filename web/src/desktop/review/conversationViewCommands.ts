import type { DesktopCommand } from "../commands/DesktopCommandProvider";
import type { DesktopPane } from "../DesktopWorkspaceController";
import {
  desktopLeaderSequence,
  DESKTOP_WORKSPACE_KEYS,
} from "../commands/workspaceShortcuts";
import { desktopConversationShown, requestDesktopWorkView } from "./desktopWorkView";

/** `␣C`: focus the Conversation, returning from Review or Tasks first. Leaving
 *  Review or Tasks is always this deliberate key; `Esc` never does it. */
export function focusConversationCommand(
  focusPane: (pane: DesktopPane) => void,
  when: () => boolean,
): DesktopCommand {
  return {
    id: "workspace.focusConversation",
    title: "Focus Conversation",
    description: "Show and focus the Conversation (from Review or Tasks too)",
    group: "Workspace",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusConversation),
    when,
    run: () => {
      if (desktopConversationShown()) {
        focusPane("conversation");
        return;
      }
      requestDesktopWorkView("conversation");
      // The transcript is shown after the view change commits.
      requestAnimationFrame(() => focusPane("conversation"));
    },
  };
}

/** Transcript-only bare keys (Follow, History/Explore, Page Index) answer
 *  only while the work pane shows the Conversation. */
export function transcriptShown(): boolean {
  return desktopConversationShown();
}

export const TRANSCRIPT_HIDDEN_REASON = "Return to the Conversation first";
