import { alpha, Box, ButtonBase, Stack, Tooltip, Typography } from "@mui/material";
import { Add, KeyboardDoubleArrowLeft, KeyboardDoubleArrowRight } from "@mui/icons-material";
import { ShortcutKeycap } from "../ShortcutKeycap";
import { ProviderIcon } from "../ProviderIcon";
import type { SessionMeta } from "../protocol";
import { DESKTOP_INSET_RADIUS, DESKTOP_SURFACE_RADIUS } from "./DesktopEmbeddedControl";
import { type DesktopPane, useDesktopWorkspace } from "./DesktopWorkspaceController";
import { DesktopShortcut } from "./commands/DesktopKeycap";
import { DESKTOP_SHORTCUTS, DESKTOP_WORKSPACE_KEYS } from "./commands/workspaceShortcuts";
import { sessionMonogram } from "./sessionMonogram";
import { isMac } from "../platform";

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

function SessionTile({
  session,
  active,
  slot,
  status,
  onPick,
}: {
  session: SessionMeta;
  active: boolean;
  slot: number;
  status: React.ReactNode;
  onPick: (id: string) => void;
}): React.JSX.Element {
  const digit = slot < 10 ? (slot === 9 ? "0" : String(slot + 1)) : null;
  return (
    <Tooltip
      placement="right"
      enterDelay={250}
      title={
        <Stack direction="row" alignItems="center" spacing={0.75}>
          <span>{session.title}</span>
          {digit && <DesktopShortcut shortcut={`Alt+${digit}`} compact />}
        </Stack>
      }
    >
      <ButtonBase
        data-desktop-session-tile={session.id}
        aria-label={session.title}
        aria-current={active ? "true" : undefined}
        aria-keyshortcuts={digit ? `${isMac ? "Option" : "Alt"}+${digit}` : undefined}
        onMouseDown={(event): void => event.preventDefault()}
        onClick={(): void => onPick(session.id)}
        sx={{
          position: "relative",
          width: 40,
          height: 40,
          flexShrink: 0,
          borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
          border: 1,
          borderColor: active
            ? (theme) => alpha(theme.palette.primary.main, 0.38)
            : "divider",
          bgcolor: active
            ? (theme) => alpha(theme.palette.primary.main, 0.13)
            : (theme) => alpha(theme.palette.background.paper, 0.46),
          color: active ? "primary.main" : "text.primary",
          transition: "background-color 120ms ease, border-color 120ms ease",
          "&:hover": {
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.48),
            bgcolor: (theme) => alpha(theme.palette.primary.main, active ? 0.16 : 0.06),
          },
          "&.Mui-focusVisible": {
            borderColor: "primary.main",
            boxShadow: (theme) => `0 0 0 2px ${alpha(theme.palette.primary.main, 0.16)}`,
          },
          // The current session also gets the edge pill used by collapsed
          // workspace switchers, so it reads at a glance down a long column.
          ...(active && {
            "&::before": {
              content: '""',
              position: "absolute",
              left: -7,
              top: 8,
              bottom: 8,
              width: 3,
              borderRadius: 99,
              bgcolor: "primary.main",
            },
          }),
        }}
      >
        <Typography
          component="span"
          sx={{ fontSize: 13, fontWeight: 650, lineHeight: 1, letterSpacing: "0.02em" }}
        >
          {sessionMonogram(session.title)}
        </Typography>
        <Box
          component="span"
          sx={{
            position: "absolute",
            top: 2,
            right: 2,
            display: "inline-flex",
            "& > *": { m: 0 },
          }}
        >
          {status}
        </Box>
        <Box
          component="span"
          sx={{
            position: "absolute",
            right: -4,
            bottom: -4,
            width: 18,
            height: 18,
            display: "grid",
            placeItems: "center",
            borderRadius: 99,
            bgcolor: "background.default",
            border: 1,
            borderColor: "divider",
            // ProviderIcon renders nothing until the installed Provider
            // contract supplies an asset; never show an empty ring.
            "&:empty": { display: "none" },
          }}
        >
          <ProviderIcon
            provider={session.provider}
            providerVersion={session.provider_version}
            providerDigest={session.provider_generation_digest}
            sx={{ fontSize: 12, width: 12, height: 12 }}
          />
        </Box>
      </ButtonBase>
    </Tooltip>
  );
}

/**
 * Collapsed Sessions: a narrow switcher instead of an empty edge. Each tile is
 * a session (monogram, live status, provider) in the same order as the
 * `Alt/Option+1…0` slots, so switching and watching agents still work while
 * the full list is folded away. The full list stays mounted (hidden) beside
 * it, so its keyboard slots and folder state are unaffected.
 */
export function DesktopSessionsRail({
  sessions,
  activeId,
  allowNewSession,
  onPick,
  onNew,
  renderStatus,
}: {
  sessions: readonly SessionMeta[];
  activeId: string | null;
  allowNewSession: boolean;
  onPick: (id: string) => void;
  onNew: () => void;
  renderStatus: (session: SessionMeta) => React.ReactNode;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
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
          border: 1,
          borderColor: "transparent",
          WebkitAppRegion: "no-drag",
          "&:hover": {
            color: "primary.main",
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.28),
            bgcolor: (theme) => alpha(theme.palette.primary.main, 0.06),
          },
          "&.Mui-focusVisible": {
            borderColor: "primary.main",
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
        bgcolor: (theme) => alpha(theme.palette.background.paper, 0.24),
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
      <Box
        sx={{
          display: "none",
          justifyContent: "center",
          pt: 1,
          "@media (display-mode: window-controls-overlay)": { display: "flex" },
        }}
      >
        {expand}
      </Box>
      <Stack
        alignItems="center"
        spacing={1.25}
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          py: 1.25,
          scrollbarWidth: "none",
          "&::-webkit-scrollbar": { display: "none" },
        }}
      >
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
                width: 40,
                height: 40,
                flexShrink: 0,
                borderRadius: `${DESKTOP_SURFACE_RADIUS}px`,
                border: 1,
                borderStyle: "dashed",
                borderColor: "divider",
                color: "text.secondary",
                "&:hover": {
                  color: "primary.main",
                  borderColor: (theme) => alpha(theme.palette.primary.main, 0.48),
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.06),
                },
              }}
            >
              <Add sx={{ fontSize: 20 }} />
            </ButtonBase>
          </Tooltip>
        )}
        {sessions.map((session, slot) => (
          <SessionTile
            key={session.id}
            session={session}
            active={session.id === activeId}
            slot={slot}
            status={renderStatus(session)}
            onPick={onPick}
          />
        ))}
      </Stack>
    </Box>
  );
}
