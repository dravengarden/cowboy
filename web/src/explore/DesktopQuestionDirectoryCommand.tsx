import { useMemo } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "../desktop/commands/DesktopCommandProvider";
import { desktopConversationShown } from "../desktop/review/desktopWorkView";

/** Explore's `P`: the Page Index of the transcript, so it answers only while
 *  the work pane shows the Conversation, not Review or Tasks. */
export function DesktopQuestionDirectoryCommand({
  available,
  toggle,
}: {
  available: boolean;
  toggle: () => void;
}): null {
  const command = useMemo<DesktopCommand>(() => ({
    id: "conversation.toggleQuestionDirectory",
    title: "Toggle Page Index",
    description: "Find and jump to a page",
    group: "Conversation",
    shortcut: "P",
    contexts: ["conversation"],
    when: () => available && desktopConversationShown(),
    disabledReason: () => !desktopConversationShown()
      ? "Return to the Conversation first"
      : "No pages are available",
    run: toggle,
  }), [available, toggle]);
  useDesktopCommand(command);
  return null;
}
