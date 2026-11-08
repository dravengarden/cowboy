import { useEffect, useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommands,
} from "../commands/DesktopCommandProvider";
import { desktopLeaderSequence } from "../commands/workspaceShortcuts";
import { desktopReviewShown } from "./desktopWorkView";
import { REVIEW_NAVIGATOR_REGION } from "./DesktopReviewNavigator";

export interface ReviewWorkbenchActions {
  readonly showNavigator: (tab: "changes" | "files" | "history" | "worktrees") => void;
  readonly openContext: () => void;
  readonly openSettings: () => void;
  readonly openComparison: () => void;
  readonly toggleUnread: () => void;
  readonly mark: () => void;
  readonly sendAndStay: () => void;
  readonly closeTab: () => void;
  readonly keepTab: () => void;
  readonly toggleNavigator: () => void;
  readonly navigatorOpen: () => void;
  readonly navigatorLeft: () => void;
  readonly navigatorTab: (offset: 1 | -1) => void;
  readonly navigatorFilter: () => void;
  readonly navigatorClose: () => void;
  readonly refresh: () => void;
  readonly hasDocument: boolean;
  readonly overlay: boolean;
}

function leader(
  id: string,
  path: string,
  title: string,
  run: () => void,
  when: () => boolean = desktopReviewShown,
): DesktopCommand {
  return {
    id,
    title,
    group: "Review",
    sequence: desktopLeaderSequence(path),
    leaderAnywhere: true,
    when,
    disabledReason: "Open Review first",
    run,
  };
}

function navigatorKey(id: string, shortcut: string, title: string, run: () => void, when?: () => boolean): DesktopCommand {
  return {
    id,
    title,
    group: "Review",
    shortcut,
    contexts: ["conversation"],
    regions: [REVIEW_NAVIGATOR_REGION],
    ...(when ? { when } : {}),
    run,
  };
}

/** The Review group's keys (`␣V…`) and the Navigator's own keys. The
 *  commands are registered once; they read the latest actions. */
export function useReviewWorkbenchCommands(actions: ReviewWorkbenchActions): void {
  const { register } = useDesktopCommands();
  const latest = useRef(actions);
  latest.current = actions;
  const commands = useMemo((): DesktopCommand[] => {
    const a = (): ReviewWorkbenchActions => latest.current;
    const withDocument = (): boolean => desktopReviewShown() && a().hasDocument;
    return [
      leader("review.changes", "VC", "Review: Changes", () => a().showNavigator("changes")),
      leader("review.files", "VF", "Review: Files", () => a().showNavigator("files")),
      leader("review.history", "VH", "Review: History", () => a().showNavigator("history")),
      leader("review.worktrees", "VW", "Review: Worktrees", () => a().showNavigator("worktrees")),
      leader("review.context", "VX", "Review: Choose what to review…", () => a().openContext()),
      leader("review.settings", "V,", "Review: Settings", () => a().openSettings()),
      leader("review.compare", "VA", "Review: Compare against…", () => a().openComparison()),
      leader("review.unread", "VU", "Review: Only unreviewed files", () => a().toggleUnread()),
      leader("review.mark", "VM", "Review: Mark reviewed", () => a().mark()),
      leader("review.send", "VS", "Review: Send to the Prompt and stay", () => a().sendAndStay()),
      leader("review.closeTab", "VQ", "Review: Close file tab", () => a().closeTab(), withDocument),
      leader("review.keepTab", "VP", "Review: Keep file tab", () => a().keepTab(), withDocument),
      leader("review.toggleNavigator", "V[", "Review: Fold the Navigator", () => a().toggleNavigator()),
      {
        id: "review.refresh",
        title: "Review: Refresh changes",
        group: "Review",
        when: desktopReviewShown,
        disabledReason: "Open Review first",
        run: () => a().refresh(),
      },
      navigatorKey("review.navigator.open", "L", "Open the focused file", () => a().navigatorOpen()),
      navigatorKey("review.navigator.left", "H", "Collapse, or go to the parent folder", () => a().navigatorLeft()),
      navigatorKey("review.navigator.previousTab", "[", "Previous Navigator tab", () => a().navigatorTab(-1)),
      navigatorKey("review.navigator.nextTab", "]", "Next Navigator tab", () => a().navigatorTab(1)),
      navigatorKey("review.navigator.filter", "/", "Filter the Navigator", () => a().navigatorFilter()),
      navigatorKey("review.navigator.close", "Escape", "Close the Navigator", () => a().navigatorClose(),
        () => a().overlay),
    ];
  }, []);
  useEffect(() => {
    const unregister = commands.map((command) => register(command));
    return () => unregister.forEach((release) => release());
  }, [commands, register]);
}
