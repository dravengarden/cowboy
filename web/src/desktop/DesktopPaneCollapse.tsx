import {
  alpha,
  Box,
  ButtonBase,
  Divider,
  ListSubheader,
  Menu,
  MenuItem,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  Add,
  FolderOpenOutlined,
  FolderOutlined,
  KeyboardDoubleArrowLeft,
  KeyboardDoubleArrowRight,
  ListAltOutlined,
} from "@mui/icons-material";
import { useRef, useState } from "react";
import { ShortcutKeycap } from "../ShortcutKeycap";
import { ProviderIcon } from "../ProviderIcon";
import type { SessionMeta } from "../protocol";
import { DESKTOP_INSET_RADIUS, DESKTOP_SURFACE_RADIUS } from "./DesktopEmbeddedControl";
import { type DesktopPane, useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { DESKTOP_SHORTCUTS, DESKTOP_WORKSPACE_KEYS } from "./commands/workspaceShortcuts";
import type { RailGroup } from "./sessionsRailGroups";

const PANE_LABEL: Record<DesktopPane, string> = {
  sessions: "Sessions",
  prompt: "Prompt",
  conversation: "Conversation",
};

const PANE_KEY: Record<DesktopPane, string> = {
  sessions: DESKTOP_WORKSPACE_KEYS.toggleSessions,
  prompt: DESKTOP_WORKSPACE_KEYS.togglePrompt,
  conversation: DESKTOP_WORKSPACE_KEYS.toggleConversation,
};

const PANE_SHORTCUT: Record<DesktopPane, string> = {
  sessions: DESKTOP_SHORTCUTS.toggleSessions,
  prompt: DESKTOP_SHORTCUTS.togglePrompt,
  conversation: DESKTOP_SHORTCUTS.toggleConversation,
};

/** Width of a collapsed Prompt/Conversation rail. */
export const DESKTOP_PANE_RAIL_WIDTH = 36;
/** Width of the collapsed Sessions rail (session tiles need a little more). */
export const DESKTOP_SESSIONS_RAIL_WIDTH = 56;

/**
 * Sessions and Prompt fold toward the left edge, Conversation toward the
 * right. The chevron always points where the pane will go.
 */
function CollapseGlyph(
  { pane, collapsed }: { pane: DesktopPane; collapsed: boolean },
): React.JSX.Element {
  const towardLeft = (pane === "conversation") === collapsed;
  const Icon = towardLeft ? KeyboardDoubleArrowLeft : KeyboardDoubleArrowRight;
  return <Icon sx={{ fontSize: 16, width: 16, height: 16, flexShrink: 0 }} />;
}

/**
 * The continuation keycap obeys the sequential-chord law: quiet/inactive at
 * rest, available while the workspace prefix is armed. Pressing Cmd/Alt+K
 * therefore lights `[` `]` `\` across the three panes, left to right.
 */
function PaneKeycap({ pane }: { pane: DesktopPane }): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const armed = workspace.mode === "command";
  return (
    <ShortcutKeycap
      keyLabel={PANE_KEY[pane]}
      variant="global"
      accent={armed}
      availability={armed ? "available" : "inactive"}
      sx={{ flexShrink: 0 }}
    />
  );
}

function PaneTooltip(
  { pane, collapsed }: { pane: DesktopPane; collapsed: boolean },
): React.JSX.Element {
  return (
    <Stack direction="row" alignItems="center" spacing={0.75}>
      <span>{collapsed ? "Expand" : "Collapse"} {PANE_LABEL[pane]}</span>
      <DesktopShortcut shortcut={PANE_SHORTCUT[pane]} compact />
    </Stack>
  );
}

/** Header control that folds its pane away. */
export function DesktopPaneCollapseButton(
  { pane, sx }: { pane: DesktopPane; sx?: object },
): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  return (
    <Tooltip title={<PaneTooltip pane={pane} collapsed={false} />} enterDelay={350}>
      <ButtonBase
        data-desktop-pane-collapse={pane}
        aria-label={`Collapse ${PANE_LABEL[pane]}`}
        aria-keyshortcuts={PANE_SHORTCUT[pane]}
        // Keep keyboard focus where it is; the command moves it if needed.
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(): void => workspace.togglePane(pane)}
        sx={{
          height: 24,
          px: 0.5,
          gap: 0.4,
          flexShrink: 0,
          borderRadius: `${DESKTOP_INSET_RADIUS}px`,
          color: "text.secondary",
          border: 1,
          borderColor: "transparent",
          transition: "background-color 120ms ease, border-color 120ms ease, color 120ms ease",
          "&:hover": {
            color: "primary.main",
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.28),
            bgcolor: (theme) => alpha(theme.palette.primary.main, 0.06),
          },
          "&.Mui-focusVisible": {
            borderColor: "primary.main",
            boxShadow: (theme) => `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
          },
          ...sx,
        }}
      >
        <CollapseGlyph pane={pane} collapsed={false} />
        <PaneKeycap pane={pane} />
      </ButtonBase>
    </Tooltip>
  );
}

/** Shared hover/focus material for the clickable collapsed rails. */
function railSx(side: "left" | "right") {
  return {
    flex: "0 0 auto",
    alignSelf: "stretch",
    display: "flex",
    flexDirection: "column",
    alignItems: "center",
    justifyContent: "flex-start",
    gap: 1,
    py: 1,
    cursor: "pointer",
    color: "text.secondary",
    bgcolor: (theme: import("@mui/material").Theme) =>
      alpha(theme.palette.background.paper, 0.24),
    [side === "left" ? "borderRight" : "borderLeft"]: 1,
    borderColor: "divider",
    transition: "background-color 120ms ease, color 120ms ease",
    "&:hover": {
      color: "primary.main",
      bgcolor: (theme: import("@mui/material").Theme) =>
        alpha(theme.palette.primary.main, 0.05),
    },
    "&.Mui-focusVisible": {
      outline: "none",
      boxShadow: (theme: import("@mui/material").Theme) =>
        `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
    },
  };
}

function RailLabel({ children }: { children: string }): React.JSX.Element {
  return (
    <Typography
      component="span"
      variant="overline"
      sx={{
        writingMode: "vertical-rl",
        fontWeight: 700,
        letterSpacing: "0.12em",
        lineHeight: 1,
        whiteSpace: "nowrap",
        userSelect: "none",
      }}
    >
      {children}
    </Typography>
  );
}

/** Small count badge stacked under a rail label. */
export function DesktopRailCount(
  { count, label }: { count: number; label: string },
): React.JSX.Element | null {
  if (count <= 0) return null;
  return (
    <Tooltip title={`${String(count)} ${label}`} placement="right">
      <Box
        component="span"
        aria-label={`${String(count)} ${label}`}
        sx={{
          minWidth: 20,
          height: 18,
          px: 0.5,
          display: "inline-grid",
          placeItems: "center",
          borderRadius: 99,
          fontSize: 11,
          fontWeight: 700,
          fontVariantNumeric: "tabular-nums",
          color: "primary.main",
          bgcolor: (theme) => alpha(theme.palette.primary.main, 0.12),
        }}
      >
        {count > 99 ? "99+" : count}
      </Box>
    </Tooltip>
  );
}

/**
 * A collapsed Prompt or Conversation. The whole strip is one target: click
 * anywhere to restore. It keeps the pane's name, its restore chord, and the
 * few signals that matter while it is out of view (queued work, agent
 * activity) so collapsing never hides state the user is waiting on.
 */
export function DesktopCollapsedPaneRail({
  pane,
  side,
  indicators,
}: {
  pane: "prompt" | "conversation";
  side: "left" | "right";
  indicators?: React.ReactNode;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  return (
    <Tooltip
      title={<PaneTooltip pane={pane} collapsed />}
      placement={side === "left" ? "right" : "left"}
      enterDelay={350}
    >
      <ButtonBase
        component="aside"
        data-desktop-collapsed-rail={pane}
        aria-label={`Expand ${PANE_LABEL[pane]}`}
        aria-keyshortcuts={PANE_SHORTCUT[pane]}
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(): void => workspace.togglePane(pane)}
        sx={{ ...railSx(side), width: DESKTOP_PANE_RAIL_WIDTH }}
      >
        <CollapseGlyph pane={pane} collapsed />
        <PaneKeycap pane={pane} />
        <RailLabel>{PANE_LABEL[pane]}</RailLabel>
        {indicators}
      </ButtonBase>
    </Tooltip>
  );
}

/** Summary used by a group's tooltip and its menu header. */
function activitySummary(group: RailGroup): string {
  const parts = [
    group.activity.working > 0 ? `${String(group.activity.working)} working` : null,
    group.activity.attention > 0
      ? `${String(group.activity.attention)} ${group.activity.attention === 1 ? "needs" : "need"} attention`
      : null,
    group.activity.live > 0 ? `${String(group.activity.live)} ready` : null,
    `${String(group.sessionCount)} ${group.sessionCount === 1 ? "session" : "sessions"}`,
  ];
  return parts.filter(Boolean).join(" · ");
}

/**
 * One badge, the most actionable count: attention (amber) beats working
 * (green). Ready and dormant sessions never badge — a rail of calm folders
 * should look calm. The full breakdown is in the tooltip and menu header.
 */
function GroupBadge({ group }: { group: RailGroup }): React.JSX.Element | null {
  const { attention, working } = group.activity;
  const count = attention > 0 ? attention : working;
  if (count === 0) return null;
  return (
    <Box
      component="span"
      aria-hidden
      sx={{
        position: "absolute",
        // Sits off the glyph's corner so the folder shape stays readable.
        top: -6,
        right: -9,
        minWidth: 15,
        height: 15,
        px: "3px",
        display: "grid",
        placeItems: "center",
        borderRadius: 99,
        fontSize: 9.5,
        fontWeight: 750,
        lineHeight: 1,
        fontVariantNumeric: "tabular-nums",
        color: "common.white",
        bgcolor: attention > 0 ? "warning.main" : "success.main",
        // A ring in the rail colour separates the badge from the glyph.
        boxShadow: (theme) => `0 0 0 2px ${theme.palette.background.default}`,
      }}
    >
      {count > 9 ? "9+" : count}
    </Box>
  );
}

function RailGroupButton({
  group,
  index,
  open,
  onOpen,
}: {
  group: RailGroup;
  index: number;
  open: boolean;
  onOpen: (anchor: HTMLElement) => void;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  // Contextual slots: only while the rail owns keyboard focus, like every
  // other region-scoped hint. Digits 1…9 open that folder directly.
  const digit = index < 9 ? String(index + 1) : null;
  const hint = digit !== null && workspace.focusedRegion === "sessions.rail";
  const Icon = group.kind === "folder"
    ? (open ? FolderOpenOutlined : FolderOutlined)
    : ListAltOutlined;
  return (
    <Tooltip
      title={open ? "" : `${group.name} · ${activitySummary(group)}`}
      placement="right"
      enterDelay={300}
    >
      <ButtonBase
        data-desktop-rail-group={group.id}
        data-desktop-item={group.id}
        data-desktop-current={group.current ? "true" : undefined}
        aria-keyshortcuts={digit ?? undefined}
        aria-label={`${group.name}: ${activitySummary(group)}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-current={group.current ? "true" : undefined}
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(event): void => onOpen(event.currentTarget)}
        sx={{
          position: "relative",
          // Fill the rail (minus its gutter) instead of a fixed width, so a
          // larger font or minimum-font-size setting can never push a label
          // past the rail's edge.
          width: "100%",
          minWidth: 0,
          minHeight: 50,
          py: 0.5,
          flexShrink: 0,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          gap: "3px",
          borderRadius: `${DESKTOP_INSET_RADIUS + 2}px`,
          color: group.current ? "primary.main" : "text.secondary",
          bgcolor: open ? "action.selected" : "transparent",
          transition: "background-color 120ms ease, color 120ms ease",
          "&:hover": { bgcolor: open ? "action.selected" : "action.hover", color: "text.primary" },
          "&.Mui-focusVisible, &:focus-visible": {
            outline: "none",
            bgcolor: open ? "action.selected" : "action.hover",
            boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.55)}`,
          },
          // The group holding the open session carries the edge pill.
          ...(group.current && {
            "&::before": {
              content: '""',
              position: "absolute",
              left: -3,
              top: 10,
              height: 20,
              width: 3,
              borderRadius: 99,
              bgcolor: "primary.main",
            },
          }),
        }}
      >
        <Box component="span" sx={{ position: "relative", display: "inline-flex" }}>
          <Icon sx={{ fontSize: 22 }} />
          <GroupBadge group={group} />
          {hint && digit && (
            <ShortcutKeycap
              keyLabel={digit}
              variant="context"
              availability="available"
              sx={{ position: "absolute", top: -7, left: -14 }}
            />
          )}
        </Box>
        <Typography
          component="span"
          sx={{
            width: "100%",
            textAlign: "center",
            fontSize: 10.5,
            lineHeight: 1.15,
            fontWeight: group.current ? 700 : 550,
            overflow: "hidden",
            textOverflow: "ellipsis",
            whiteSpace: "nowrap",
          }}
        >
          {group.name}
        </Typography>
      </ButtonBase>
    </Tooltip>
  );
}

/** The opened group: real titles, subfolder headings, slots and status. */
function RailGroupMenu({
  group,
  anchor,
  activeId,
  slots,
  onClose,
  onExited,
  onPick,
  onShowAll,
  renderStatus,
}: {
  group: RailGroup | null;
  anchor: HTMLElement | null;
  activeId: string | null;
  slots: ReadonlyMap<string, number>;
  /** `back`: the user stepped back out (Esc / h) — return focus to the rail. */
  onClose: (back: boolean) => void;
  /** The close transition finished and the menu items are gone. */
  onExited: () => void;
  onPick: (id: string) => void;
  onShowAll: () => void;
  renderStatus: (session: SessionMeta) => React.ReactNode;
}): React.JSX.Element {
  // Vim keys inside the menu, matching the list it stands in for: j/k move,
  // l opens, h steps back to the rail. Captured before MUI's first-letter
  // type-ahead, which would otherwise treat j/k/h/l as item searches.
  const onListKeyDown = (event: React.KeyboardEvent<HTMLElement>): void => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    if (!["j", "k", "h", "l"].includes(key)) return;
    event.preventDefault();
    event.stopPropagation();
    if (key === "h") {
      onClose(true);
      return;
    }
    const items = [...event.currentTarget.querySelectorAll<HTMLElement>(
      "[role='menuitem']:not([aria-disabled='true'])",
    )];
    const current = items.indexOf(document.activeElement as HTMLElement);
    if (key === "l") {
      items[current]?.click();
      return;
    }
    const next = key === "j"
      ? Math.min(items.length - 1, current + 1)
      : Math.max(0, current < 0 ? 0 : current - 1);
    items[next]?.focus();
  };
  const empty = !group || group.sections.every((section) => section.sessions.length === 0);
  return (
    <Menu
      open={group !== null && anchor !== null}
      anchorEl={anchor}
      onClose={(_event, reason): void => onClose(reason === "escapeKeyDown")}
      // Focus is placed explicitly: back to the rail on Esc/h, into the
      // Prompt after opening a session (MUI would pull it back to the rail).
      disableRestoreFocus
      anchorOrigin={{ vertical: "top", horizontal: "right" }}
      transformOrigin={{ vertical: "top", horizontal: "left" }}
      slotProps={{
        paper: {
          sx: {
            ml: 1,
            width: 320,
            maxHeight: "70vh",
            borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
            border: 1,
            borderColor: "divider",
          },
        },
        list: { dense: true, sx: { py: 0.5 }, onKeyDownCapture: onListKeyDown },
        transition: { onExited },
      }}
    >
      {group && (
        <ListSubheader
          disableSticky
          sx={{ lineHeight: 1.3, pt: 1, pb: 0.75, bgcolor: "transparent" }}
        >
          <Typography component="div" variant="body2" sx={{ fontWeight: 700, color: "text.primary" }}>
            {group.name}
          </Typography>
          <Typography component="div" variant="caption" color="text.secondary">
            {activitySummary(group)}
          </Typography>
        </ListSubheader>
      )}
      {group?.sections.flatMap((section) => {
        const indent = section.depth + (section.title ? 1 : 0);
        const items: React.ReactNode[] = [];
        if (section.title) {
          items.push(
            <ListSubheader
              key={`heading:${section.folder ?? ""}`}
              disableSticky
              sx={{
                display: "flex",
                alignItems: "center",
                gap: 0.75,
                lineHeight: 2.2,
                pl: 2 + section.depth * 1.5,
                bgcolor: "transparent",
                fontSize: 12,
                fontWeight: 650,
              }}
            >
              <FolderOutlined sx={{ fontSize: 15 }} />
              {section.title}
            </ListSubheader>,
          );
        }
        for (const session of section.sessions) {
          const slot = slots.get(session.id);
          const digit = slot !== undefined && slot < 10 ? (slot === 9 ? "0" : String(slot + 1)) : null;
          items.push(
            <MenuItem
              key={session.id}
              selected={session.id === activeId}
              data-desktop-rail-session={session.id}
              onClick={(): void => {
                onPick(session.id);
                onClose(false);
              }}
              sx={{
                gap: 1,
                minHeight: 36,
                pl: 2 + indent * 1.5,
                // The open session reads like the current row in the list.
                "&.Mui-focusVisible, &:focus-visible": {
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08),
                  boxShadow: (theme) => `inset 2px 0 0 ${theme.palette.primary.main}`,
                },
                "&.Mui-selected": {
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.13),
                  color: "primary.main",
                  "& .MuiTypography-root": { fontWeight: 650 },
                },
              }}
            >
              <Box component="span" sx={{ width: 12, display: "inline-flex", justifyContent: "center", flexShrink: 0 }}>
                {renderStatus(session)}
              </Box>
              <ProviderIcon
                provider={session.provider}
                providerVersion={session.provider_version}
                providerDigest={session.provider_generation_digest}
                sx={{ fontSize: 16, width: 16, height: 16, flexShrink: 0 }}
              />
              <Typography variant="body2" noWrap sx={{ flex: 1, minWidth: 0 }}>
                {session.title}
              </Typography>
              {digit && <DesktopShortcut shortcut={`Alt+${digit}`} compact quiet />}
            </MenuItem>,
          );
        }
        return items;
      })}
      {group && empty && (
        <MenuItem disabled sx={{ fontStyle: "italic", fontSize: 13 }}>
          No sessions in this folder
        </MenuItem>
      )}
      {group && <Divider sx={{ my: 0.5 }} />}
      {group && (
        <MenuItem
          onClick={(): void => {
            onClose(false);
            onShowAll();
          }}
          sx={{ gap: 1, minHeight: 34, color: "text.secondary", fontSize: 13 }}
        >
          <KeyboardDoubleArrowRight sx={{ fontSize: 16 }} />
          <Box component="span" sx={{ flex: 1 }}>Show all sessions</Box>
          <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.toggleSessions} compact quiet />
        </MenuItem>
      )}
      {group && (
        // The menu's own key map, since it owns the keyboard while open.
        <Box
          aria-hidden
          sx={{
            display: "flex",
            alignItems: "center",
            gap: 0.5,
            px: 2,
            pt: 0.5,
            pb: 0.75,
            fontSize: 11,
            color: "text.disabled",
          }}
        >
          <ShortcutKeycap keyLabel="J/K" variant="modal" />
          <span>move</span>
          <ShortcutKeycap keyLabel="Enter" variant="modal" sx={{ ml: 0.75 }} />
          <span>open</span>
          <ShortcutKeycap keyLabel="H" variant="modal" sx={{ ml: 0.75 }} />
          <span>back</span>
        </Box>
      )}
    </Menu>
  );
}

/**
 * Collapsed Sessions. A 56 px column cannot show titles, so it shows the
 * user's own structure: one entry per top-level folder (plus Unfiled), each
 * with a label and a single actionable badge (needs attention, else working).
 * Opening an entry lists its sessions with real titles, subfolders, status
 * and `Alt/Option+1…0` slots. The group holding the open session carries the
 * edge pill. The full list stays mounted (hidden) beside it, so keyboard
 * slots and folder state are unaffected.
 */
export function DesktopSessionsRail({
  groups,
  activeId,
  slots,
  allowNewSession,
  onPick,
  onNew,
  renderStatus,
}: {
  groups: readonly RailGroup[];
  activeId: string | null;
  slots: ReadonlyMap<string, number>;
  allowNewSession: boolean;
  onPick: (id: string) => void;
  onNew: () => void;
  renderStatus: (session: SessionMeta) => React.ReactNode;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const [menu, setMenu] = useState<{ id: string; anchor: HTMLElement } | null>(null);
  // Esc/h steps back to the folder that opened the menu. Focus moves only
  // after the menu has unmounted; earlier, its focus trap or the vanishing
  // item would drop focus to the region instead.
  const returnFocus = useRef<HTMLElement | null>(null);
  const openGroup = menu ? groups.find((group) => group.id === menu.id) ?? null : null;
  const expand = (
    <Tooltip title={<PaneTooltip pane="sessions" collapsed />} placement="right" enterDelay={350}>
      <ButtonBase
        data-desktop-collapsed-rail="sessions"
        aria-label="Expand Sessions"
        aria-keyshortcuts={DESKTOP_SHORTCUTS.toggleSessions}
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(): void => workspace.togglePane("sessions")}
        sx={{
          height: 30,
          px: 0.6,
          gap: 0.4,
          borderRadius: `${DESKTOP_INSET_RADIUS}px`,
          color: "text.secondary",
          WebkitAppRegion: "no-drag",
          "&:hover": { color: "primary.main", bgcolor: "action.hover" },
          "&.Mui-focusVisible": {
            boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
          },
        }}
      >
        <CollapseGlyph pane="sessions" collapsed />
        <PaneKeycap pane="sessions" />
      </ButtonBase>
    </Tooltip>
  );
  return (
    <Box
      component="nav"
      aria-label="Sessions (collapsed)"
      data-desktop-sessions-rail
      // The collapsed rail is the visible Sessions pane: Cmd/Alt+K S focuses
      // it (instead of unfolding the list), so the layout choice survives.
      data-desktop-pane="sessions"
      sx={{
        width: DESKTOP_SESSIONS_RAIL_WIDTH,
        // Nothing inside may widen or scroll the rail sideways.
        overflow: "clip",
        flexShrink: 0,
        height: "100%",
        display: "flex",
        flexDirection: "column",
        borderRight: 1,
        borderColor: "divider",
        bgcolor: "background.default",
      }}
    >
      <Box
        data-desktop-sessions-rail-head
        sx={{
          // Same height as the top bar so the two hairlines meet.
          minHeight: 44,
          flexShrink: 0,
          display: "grid",
          placeItems: "center",
          borderBottom: 1,
          borderColor: "divider",
          // Installed PWA: macOS window controls overlay this corner. Make it
          // a drag handle and move the expand control into the list below.
          "@media (display-mode: window-controls-overlay)": {
            WebkitAppRegion: "drag",
            "& > *": { display: "none" },
          },
        }}
      >
        {expand}
      </Box>
      <Stack
        data-desktop-region="sessions.rail"
        tabIndex={-1}
        alignItems="center"
        spacing={0.5}
        sx={{
          outline: "none",
          px: "3px",
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          py: 1,
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
        <Box
          sx={{
            display: "none",
            "@media (display-mode: window-controls-overlay)": { display: "flex" },
          }}
        >
          {expand}
        </Box>
        {allowNewSession && (
          <Tooltip
            placement="right"
            enterDelay={250}
            title={
              <Stack direction="row" alignItems="center" spacing={0.75}>
                <span>New Session</span>
                <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.newSession} compact />
              </Stack>
            }
          >
            <ButtonBase
              aria-label="New Session"
              aria-keyshortcuts={DESKTOP_SHORTCUTS.newSession}
              onMouseDown={(event): void => event.preventDefault()}
              onClick={onNew}
              sx={{
                width: 36,
                height: 36,
                flexShrink: 0,
                borderRadius: `${DESKTOP_INSET_RADIUS + 2}px`,
                color: "text.secondary",
                "&:hover": { color: "primary.main", bgcolor: "action.hover" },
                "&.Mui-focusVisible": {
                  boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
                },
              }}
            >
              <Add sx={{ fontSize: 22 }} />
            </ButtonBase>
          </Tooltip>
        )}
        {groups.length > 0 && (
          <Box aria-hidden sx={{ width: 24, height: "1px", bgcolor: "divider", my: 0.5, flexShrink: 0 }} />
        )}
        {groups.map((group, index) => (
          <RailGroupButton
            key={group.id}
            group={group}
            index={index}
            open={menu?.id === group.id}
            onOpen={(anchor): void => setMenu({ id: group.id, anchor })}
          />
        ))}
      </Stack>
      <RailGroupMenu
        group={openGroup}
        anchor={menu?.anchor ?? null}
        activeId={activeId}
        slots={slots}
        onClose={(back): void => {
          returnFocus.current = back ? menu?.anchor ?? null : null;
          setMenu(null);
        }}
        onExited={(): void => {
          returnFocus.current?.focus({ preventScroll: true });
          returnFocus.current = null;
        }}
        onPick={(id): void => {
          onPick(id);
          // Opening a session from the rail lands in its work surface, as
          // opening one from the list does; a collapsed Prompt stays folded.
          const target = workspace.collapsedPanes.prompt
            ? "conversation.transcript"
            : "prompt.composer";
          requestAnimationFrame(() => workspace.focusRegion(target));
        }}
        onShowAll={(): void => workspace.togglePane("sessions")}
        renderStatus={renderStatus}
      />
    </Box>
  );
}
