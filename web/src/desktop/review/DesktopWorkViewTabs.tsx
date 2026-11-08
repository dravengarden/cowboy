import { Tab, Tabs } from "@mui/material";
import { desktopSize } from "../../surface/desktopSize";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { DESKTOP_INSET_RADIUS } from "../DesktopEmbeddedControl";
import {
  DESKTOP_SHORTCUTS,
  DESKTOP_WORKSPACE_KEYS,
} from "../commands/workspaceShortcuts";
import type { DesktopWorkView } from "./desktopWorkView";

const VIEWS: readonly {
  value: DesktopWorkView;
  label: string;
  leaderKey: string;
  shortcut: string;
}[] = [
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

/** The work pane's title: its two views, each with the leader slot that
 *  shows it (`␣C`, `␣V`). Both views stay mounted behind the tabs. */
export function DesktopWorkViewTabs({
  view,
  onChange,
}: {
  view: DesktopWorkView;
  onChange: (view: DesktopWorkView) => void;
}): React.JSX.Element {
  return (
    <Tabs
      data-desktop-work-view-tabs
      value={view}
      onChange={(_event, next: DesktopWorkView) => onChange(next)}
      aria-label="Work pane view"
      sx={{
        minHeight: desktopSize(28),
        flexShrink: 0,
        "& .MuiTabs-indicator": { height: 2 },
      }}
    >
      {VIEWS.map((item) => (
        <Tab
          key={item.value}
          value={item.value}
          data-desktop-work-view={item.value}
          aria-keyshortcuts={item.shortcut}
          disableRipple
          label={item.label}
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
}
