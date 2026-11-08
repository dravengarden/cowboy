import { useEffect, useMemo, useRef } from "react";
import {
  type DesktopCommand,
  useDesktopCommands,
} from "../commands/DesktopCommandProvider";
import { desktopLeaderSequence } from "../commands/workspaceShortcuts";
import { desktopReviewShown } from "./desktopWorkView";
import { REVIEW_NAVIGATOR_REGION } from "./DesktopReviewNavigator";

export interface ReviewWorkbenchActions {
  readonly showNavigator: (tab: "changes" | "files" | "history" | "worktrees" | "pulls") => void;
  readonly openContext: () => void;
  readonly inspector: { show: () => void; toggle: () => void; runCheck: () => void };
  readonly openSettings: () => void;
  readonly openComparison: () => void;
  readonly toggleUnread: () => void;
  readonly mark: () => void;
  readonly sendAndStay: () => void;
  readonly delegate: (kind: "explain" | "review") => void;
  readonly closeTab: () => void;
  readonly keepTab: () => void;
  readonly toggleNavigator: () => void;
  readonly navigatorOpen: () => void;
  readonly navigatorLeft: () => void;
  readonly navigatorTab: (offset: 1 | -1) => void;
  readonly navigatorFilter: () => void;
  readonly navigatorClose: () => void;
  readonly refresh: () => void;
  readonly toggleDiffLayout: () => void;
  readonly toggleDiffContext: () => void;
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
      leader("review.pulls", "VG", "Review: Pull request", () => a().showNavigator("pulls")),
      leader("review.context", "VX", "Review: Choose what to review…", () => a().openContext()),
      leader("review.settings", "V,", "Review: Settings", () => a().openSettings()),
      leader("review.compare", "VA", "Review: Compare against…", () => a().openComparison()),
      leader("review.unread", "VU", "Review: Only unreviewed files", () => a().toggleUnread()),
      leader("review.mark", "VM", "Review: Mark reviewed", () => a().mark()),
      leader("review.send", "VS", "Review: Send to the Prompt and stay", () => a().sendAndStay()),
      leader("review.explain", "VE", "Review: Explain in a fork", () => a().delegate("explain")),
      leader("review.delegate", "VR", "Review: Ask a fork to review this file", () => a().delegate("review")),
      leader("review.closeTab", "VQ", "Review: Close file tab", () => a().closeTab(), withDocument),
      leader("review.keepTab", "VP", "Review: Keep file tab", () => a().keepTab(), withDocument),
      leader("review.toggleNavigator", "V[", "Review: Fold the Navigator", () => a().toggleNavigator()),
      leader("review.diffLayout", "VD", "Review: Unified or split diff", () => a().toggleDiffLayout()),
      leader("review.diffContext", "VZ", "Review: Whole file or the patch's context", () => a().toggleDiffContext()),
      leader("review.outline", "VO", "Review: Outline in the Inspector", () => a().inspector.show()),
      leader("review.toggleInspector", "V]", "Review: Fold the Inspector", () => a().inspector.toggle()),
      leader("review.check", "VK", "Review: Run a project check", () => a().inspector.runCheck()),
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
