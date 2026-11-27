// Unified calm Mobile Composer entry combining Branches & lineage with
// Background tasks in the ordinary scrollable action track (no right-edge pin,
// no divider border, no oversized badge). Vanishes (0px) when a session has
// neither branches nor background tasks.

import { memo, useState } from "react";
import { Box, IconButton, Tooltip } from "@mui/material";
import { LayersOutlined } from "@mui/icons-material";
import { openBackgroundTask } from "../backgroundTaskInspectorState";
import { releaseMobileComposerFocus } from "../composer/mobileComposerFocus";
import { useExploreSessionState } from "../explore/exploreStore";
import { GitForkIcon } from "../GitForkIcon";
import { BranchSwitcherModal, useBranchRelationCount } from "../LineageBranchSwitcher";
import { useBackgroundTasks } from "../useBackgroundTasks";
import { MobileTasksSheet } from "./MobileTasksSheet";
import { resolveMobileActivityState, taskTabSummary } from "./taskTree";

export const MobileSessionActivityButton = memo(function MobileSessionActivityButton({
  sessionId,
  showTasks,
  onBeforeOpen,
}: {
  sessionId: string;
  showTasks: boolean;
  onBeforeOpen?: () => void;
}): React.JSX.Element | null {
  const relationCount = useBranchRelationCount(sessionId);
  const background = useBackgroundTasks(sessionId);
  const summary = taskTabSummary(background.tasks, background.backgroundActivity);
  const state = resolveMobileActivityState(summary, relationCount, showTasks);
  const projection = useExploreSessionState(sessionId).projection;
  const [legacyBranchOpen, setLegacyBranchOpen] = useState(false);

  if (!state.visible) return null;

  const iconColor = state.dotTone === "warning"
    ? "warning.main"
    : state.dotTone === "error"
    ? "error.main"
    : state.dotTone === "active"
    ? "primary.main"
    : "text.secondary";

  const useStandaloneBranchModal = !showTasks && state.hasBranches;

  return (
    <>
      <Tooltip title={state.label}>
        <IconButton
          aria-label={state.label}
          aria-haspopup="dialog"
          data-mobile-session-activity={sessionId}
          data-mobile-branch-switcher={state.hasBranches ? "" : undefined}
          data-mobile-tasks-entry={state.hasTasks ? sessionId : undefined}
          data-mobile-tasks-tone={state.hasTasks ? (state.dotTone ?? "quiet") : undefined}
          onPointerDown={(event): void => {
            if (event.pointerType === "touch") releaseMobileComposerFocus();
          }}
          onClick={(event): void => {
            releaseMobileComposerFocus();
            onBeforeOpen?.();
            if (useStandaloneBranchModal) {
              setLegacyBranchOpen(true);
            } else {
              openBackgroundTask(sessionId, null, "bar", event.currentTarget, state.defaultTab);
            }
            event.currentTarget.blur();
          }}
          sx={{
            position: "relative",
            flexShrink: 0,
            width: 44,
            height: 44,
            color: iconColor,
          }}
        >
          {state.hasBranches
            ? <GitForkIcon fontSize="small" />
            : <LayersOutlined fontSize="small" />}
          {state.dotTone && (
            <Box
              component="span"
              data-mobile-activity-dot={state.dotTone}
              sx={(theme) => ({
                position: "absolute",
                top: 9,
                right: 9,
                width: 7,
                height: 7,
                borderRadius: "50%",
                bgcolor: state.dotTone === "warning"
                  ? theme.palette.warning.main
                  : state.dotTone === "error"
                  ? theme.palette.error.main
                  : theme.palette.primary.main,
                ...(state.dotTone === "active" && {
                  "@keyframes cowboyActivityDotPulse": {
                    "0%, 100%": { opacity: 1 },
                    "50%": { opacity: 0.45 },
                  },
                  animation: "cowboyActivityDotPulse 1.8s ease-in-out infinite",
                  "@media (prefers-reduced-motion: reduce)": { animation: "none" },
                }),
              })}
            />
          )}
        </IconButton>
      </Tooltip>
      {useStandaloneBranchModal && (
        <BranchSwitcherModal
          sessionId={sessionId}
          open={legacyBranchOpen}
          desktop={false}
          onClose={(): void => setLegacyBranchOpen(false)}
        />
      )}
      {!useStandaloneBranchModal && projection === "explore" && (
        <MobileTasksSheet sessionId={sessionId} background={background} />
      )}
    </>
  );
});
