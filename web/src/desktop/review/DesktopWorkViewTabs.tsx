import { memo } from "react";
import { alpha, Box, Tab, Tabs } from "@mui/material";
import { desktopSize } from "../../surface/desktopSize";
import type { SessionTasksAvailability } from "../../tasks/taskEvents";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
} from "../commands/workspaceShortcuts";
import type { DesktopWorkView } from "./desktopWorkView";

interface TabDef {
  value: DesktopWorkView;
  label: string;
  leaderKey: string;
  shortcut: string;
}

const BASE_VIEWS: readonly TabDef[] = [
  {
    value: "conversation",
    label: "Conversation",
    leaderKey: DESKTOP_WORKSPACE_KEYS.focusConversation,
    shortcut: DESKTOP_SHORTCUTS.focusConversation,
  },
  {
    value: "review",
    label: "Review",
    leaderKey: DESKTOP_WORKSPACE_KEYS.review,
    shortcut: DESKTOP_SHORTCUTS.review,
  },
];

const TASKS_VIEW: TabDef = {
  value: "tasks",
  label: "Tasks",
  leaderKey: DESKTOP_WORKSPACE_KEYS.focusCalls,
  shortcut: DESKTOP_SHORTCUTS.focusCalls,
};

function TasksTabBadge({
  tasks,
}: {
  tasks: SessionTasksAvailability;
}): React.JSX.Element | null {
  const tone = tasks.needsYouCount > 0
    ? "warning"
    : tasks.failedUnseenCount > 0
    ? "error"
    : tasks.activeCount > 0
    ? "info"
    : null;
  if (!tone) return null;
  const text = tasks.needsYouCount > 0
    ? `⚠ ${String(tasks.needsYouCount)}`
    : tasks.failedUnseenCount > 0
    ? `✕ ${String(tasks.failedUnseenCount)}`
    : String(tasks.activeCount);
  return (
    <Box
      component="span"
      data-desktop-tasks-tab-badge={tone}
      sx={(theme) => ({
        display: "inline-flex",
        alignItems: "center",
        px: 0.6,
        height: desktopSize(16),
        borderRadius: 999,
        fontSize: "0.625rem",
        fontWeight: 750,
        lineHeight: 1,
        fontVariantNumeric: "tabular-nums",
        color: theme.palette[tone].main,
        bgcolor: alpha(theme.palette[tone].main, theme.palette.mode === "dark" ? 0.22 : 0.14),
      })}
    >
      {text}
    </Box>
  );
}

/** The work pane's title: Conversation (`␣C`), Review (`␣V`), and Tasks (`␣G`)
 *  when the session has background tasks. Mounted views stay alive behind the tabs. */
export const DesktopWorkViewTabs = memo(function DesktopWorkViewTabs({
  view,
  onChange,
  tasks,
}: {
  view: DesktopWorkView;
  onChange: (view: DesktopWorkView) => void;
  tasks?: SessionTasksAvailability | undefined;
}): React.JSX.Element {
  const showTasks = Boolean(tasks?.hasTasks || view === "tasks");
  const views = showTasks ? [...BASE_VIEWS, TASKS_VIEW] : BASE_VIEWS;
  const safeValue = showTasks || view !== "tasks" ? view : "conversation";
  return (
    <Tabs
      data-desktop-work-view-tabs
      value={safeValue}
      onChange={(_event, next: DesktopWorkView) => onChange(next)}
      aria-label="Work pane view"
      sx={{
        minHeight: desktopSize(28),
        flexShrink: 0,
        "& .MuiTabs-indicator": { height: 2 },
      }}
    >
      {views.map((item) => (
        <Tab
          key={item.value}
          value={item.value}
          data-desktop-work-view={item.value}
          aria-keyshortcuts={item.shortcut}
          disableRipple
          label={
            item.value === "tasks" && tasks
              ? (
                <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.6 }}>
                  {item.label}
                  <TasksTabBadge tasks={tasks} />
                </Box>
              )
              : item.label
          }
          icon={<LeaderKeycap leaderKey={item.leaderKey} />}
          iconPosition="end"
          sx={{
            minHeight: desktopSize(28),
            minWidth: 0,
            px: 1,
            py: 0,
            mr: 0.5,
            gap: 0.75,
            borderRadius: `${String(DESKTOP_INSET_RADIUS)}px`,
            textTransform: "none",
            fontSize: "0.75rem",
            fontWeight: 650,
            letterSpacing: "0.02em",
            color: "text.secondary",
            "&.Mui-selected": { color: "text.primary" },
            "& .MuiTab-icon": { m: 0 },
          }}
        />
      ))}
    </Tabs>
  );
});
