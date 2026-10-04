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
import { useState } from "react";
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
        top: -4,
        right: -6,
        minWidth: 16,
        height: 16,
        px: "3px",
        display: "grid",
        placeItems: "center",
        borderRadius: 99,
        fontSize: 10,
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
  open,
  onOpen,
}: {
  group: RailGroup;
  open: boolean;
  onOpen: (anchor: HTMLElement) => void;
}): React.JSX.Element {
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
        aria-label={`${group.name}: ${activitySummary(group)}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-current={group.current ? "true" : undefined}
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(event): void => onOpen(event.currentTarget)}
        sx={{
          position: "relative",
          width: 48,
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
          "&.Mui-focusVisible": {
            boxShadow: (theme) => `inset 0 0 0 2px ${alpha(theme.palette.primary.main, 0.5)}`,
          },
          // The group holding the open session carries the edge pill.
          ...(group.current && {
            "&::before": {
              content: '""',
              position: "absolute",
              left: -4,
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
        </Box>
        <Typography
          component="span"
          sx={{
            maxWidth: 46,
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
  onPick,
  onShowAll,
  renderStatus,
}: {
  group: RailGroup | null;
  anchor: HTMLElement | null;
  activeId: string | null;
  slots: ReadonlyMap<string, number>;
  onClose: () => void;
  onPick: (id: string) => void;
  onShowAll: () => void;
  renderStatus: (session: SessionMeta) => React.ReactNode;
}): React.JSX.Element {
  const empty = !group || group.sections.every((section) => section.sessions.length === 0);
  return (
    <Menu
      open={group !== null && anchor !== null}
      anchorEl={anchor}
      onClose={onClose}
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
        list: { dense: true, sx: { py: 0.5 } },
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
                onClose();
              }}
              sx={{
                gap: 1,
                minHeight: 36,
                pl: 2 + indent * 1.5,
                // The open session reads like the current row in the list.
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
            onClose();
            onShowAll();
          }}
          sx={{ gap: 1, minHeight: 34, color: "text.secondary", fontSize: 13 }}
        >
          <KeyboardDoubleArrowRight sx={{ fontSize: 16 }} />
          <Box component="span" sx={{ flex: 1 }}>Show all sessions</Box>
          <DesktopShortcut shortcut={DESKTOP_SHORTCUTS.toggleSessions} compact quiet />
        </MenuItem>
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
      sx={{
        width: DESKTOP_SESSIONS_RAIL_WIDTH,
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
        alignItems="center"
        spacing={0.5}
        sx={{
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
        {groups.map((group) => (
          <RailGroupButton
            key={group.id}
            group={group}
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
        onClose={(): void => setMenu(null)}
        onPick={onPick}
        onShowAll={(): void => workspace.togglePane("sessions")}
        renderStatus={renderStatus}
      />
    </Box>
  );
}
