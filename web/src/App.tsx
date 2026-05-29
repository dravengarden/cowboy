import { useState } from "react";
import {
  AppBar,
  Box,
  Button,
  Chip,
  Dialog,
  DialogContent,
  DialogTitle,
  Drawer,
  IconButton,
  List,
  ListItemButton,
  ListItemText,
  MenuItem,
  Stack,
  TextField,
  Toolbar,
  Typography,
  useMediaQuery,
  useTheme,
} from "@mui/material";
import { Add, Circle, Close, Menu as MenuIcon, Settings as SettingsIcon } from "@mui/icons-material";
import { Composer } from "./Composer";
import { Transcript } from "./Transcript";
import {
  PROVIDERS,
  type SessionMeta,
  type SessionOrigin,
  type Status,
} from "./protocol";
import { send, useStore } from "./store";
import { Settings } from "./Settings";
import type { Mode as ThemeMode } from "./theme";

// Sidebar width: fluid clamp instead of a fixed pixel value so the panel
// grows on wide displays and shrinks on narrow ones without media-query
// staircase steps. 240px is the floor (a list row stays readable at that
// width), 22vw is the natural scale, 360px is the ceiling (any wider and
// the list looks empty next to the transcript). The mobile Drawer keeps
// its own width because the slide-in panel is meant to feel temporary,
// not adapt to viewport.
const SIDEBAR_WIDTH = "clamp(240px, 22vw, 360px)";
const MOBILE_DRAWER_WIDTH = "min(86vw, 360px)";

function statusColor(s: Status): string {
  switch (s) {
    case "running":
      return "success.main";
    case "busy":
      return "warning.main";
    case "starting":
      return "info.main";
    default:
      return "error.main";
  }
}

function originLabel(o: SessionOrigin | undefined): string {
  // Default to "api" matches the daemon's SessionOrigin default; older
  // daemons that predate the field also fall through to here.
  switch (o ?? "api") {
    case "zed":
      return "Zed";
    case "web":
      return "Web";
    default:
      return "API";
  }
}

function originColor(o: SessionOrigin | undefined): "primary" | "secondary" | "default" {
  switch (o ?? "api") {
    case "zed":
      return "primary";
    case "web":
      return "secondary";
    default:
      return "default";
  }
}

function SessionList({
  sessions,
  activeId,
  onPick,
  onNew,
}: {
  sessions: SessionMeta[];
  activeId: string | null;
  onPick: (id: string) => void;
  onNew: () => void;
}): React.JSX.Element {
  return (
    <Stack sx={{ height: "100%" }}>
      <Box sx={{ p: 1 }}>
        <Button fullWidth variant="outlined" startIcon={<Add />} onClick={onNew}>
          New session
        </Button>
      </Box>
      <List dense sx={{ flex: 1, overflowY: "auto" }}>
        {sessions.map((s) => (
          <ListItemButton
            key={s.id}
            selected={s.id === activeId}
            onClick={(): void => onPick(s.id)}
            sx={{ pr: 1 }}
          >
            <Circle sx={{ fontSize: 10, mr: 1, color: statusColor(s.status) }} />
            <ListItemText
              primary={
                <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
                  <Typography variant="body2" noWrap sx={{ minWidth: 0 }}>
                    {s.provider}
                  </Typography>
                  <Chip
                    size="small"
                    label={originLabel(s.origin)}
                    color={originColor(s.origin)}
                    sx={{ height: 16, fontSize: 10, "& .MuiChip-label": { px: 0.75 } }}
                  />
                </Stack>
              }
              secondary={s.cwd}
              slotProps={{
                primary: { component: "div" },
                secondary: { noWrap: true, variant: "caption" },
              }}
            />
            <IconButton
              size="small"
              edge="end"
              aria-label={`delete session ${s.id}`}
              onClick={(e): void => {
                e.stopPropagation();
                if (
                  window.confirm(
                    `Delete this ${originLabel(s.origin)} session? Any in-flight turn is cancelled. The agent transcript on this session will be lost (in-memory only in v1).`,
                  )
                ) {
                  send({ type: "delete_session", session_id: s.id });
                }
              }}
              sx={{ ml: 0.5 }}
            >
              <Close fontSize="inherit" />
            </IconButton>
          </ListItemButton>
        ))}
        {sessions.length === 0 && (
          <Typography variant="body2" color="text.secondary" sx={{ p: 2, textAlign: "center" }}>
            No sessions yet.
          </Typography>
        )}
      </List>
    </Stack>
  );
}

// Hard-coded workspace choices for v0. Each entry's `value` is what the
// daemon receives as `cwd`; the resolver in supervisor.rs honours absolute
// paths as-is and joins relative ones to `--workspace-root` (defaults to
// `/home/draven`). To expose more roots later, either bump this list or
// fetch a list from the daemon.
const WORKING_DIRS = [
  { value: "columbus", label: "columbus", help: "/home/draven/columbus" },
  { value: "/etc/nixos", label: "/etc/nixos", help: "NixOS host config" },
] as const;

function NewSessionDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const [provider, setProvider] = useState<string>(PROVIDERS[0]);
  const [cwd, setCwd] = useState<string>(WORKING_DIRS[0].value);
  return (
    <Dialog open={open} onClose={onClose} fullWidth maxWidth="xs">
      <DialogTitle>New session</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          <TextField
            select
            label="Provider"
            value={provider}
            onChange={(e): void => setProvider(e.target.value)}
          >
            {PROVIDERS.map((p) => (
              <MenuItem key={p} value={p}>
                {p}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            select
            label="Working directory"
            value={cwd}
            onChange={(e): void => setCwd(e.target.value)}
            helperText={WORKING_DIRS.find((w) => w.value === cwd)?.help ?? ""}
          >
            {WORKING_DIRS.map((w) => (
              <MenuItem key={w.value} value={w.value}>
                {w.label}
              </MenuItem>
            ))}
          </TextField>
          <Button
            variant="contained"
            onClick={(): void => {
              send({ type: "new_session", provider, cwd });
              onClose();
            }}
          >
            Create
          </Button>
        </Stack>
      </DialogContent>
    </Dialog>
  );
}

export function App({
  themeMode,
  onSetThemeMode,
}: {
  themeMode: ThemeMode;
  onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
  const { connected, sessions, timelines } = useStore();
  const theme = useTheme();
  const mobile = useMediaQuery(theme.breakpoints.down("sm"));
  // BottomSheet (Drawer anchor=bottom) on phones + portrait iPad; centred
  // Dialog on landscape iPad and desktop. md breakpoint is 900px — portrait
  // iPad is ~820 wide so it falls into BottomSheet, landscape iPad 1180 →
  // Dialog. Matches the iOS Settings idiom.
  const bottomSheet = useMediaQuery(theme.breakpoints.down("md"));
  const [activeId, setActiveId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Default to the first session once one exists.
  const active = sessions.find((s) => s.id === activeId) ?? sessions[0] ?? null;

  function pick(id: string): void {
    setActiveId(id);
    setDrawerOpen(false);
  }

  const list = (
    <SessionList
      sessions={sessions}
      activeId={active?.id ?? null}
      onPick={pick}
      onNew={(): void => setDialogOpen(true)}
    />
  );

  // Brand toolbar for the desktop sidebar. Uses MUI's own `<Toolbar>` so
  // its height comes from `theme.mixins.toolbar` (responsive across
  // breakpoints) and stays in lockstep with the AppBar's Toolbar on the
  // right pane — no hardcoded pixel value. On mobile the sidebar is a
  // Drawer and the brand goes in the AppBar instead.
  const sidebarHeader = (
    <Toolbar
      variant="dense"
      sx={{ borderBottom: 1, borderColor: "divider", flexShrink: 0 }}
    >
      <Typography variant="subtitle1" noWrap sx={{ fontWeight: 500 }}>
        🤠 cowboy
      </Typography>
    </Toolbar>
  );

  return (
    <Box sx={{ display: "flex", height: "100%", width: "100%" }}>
      {mobile ? (
        <Drawer open={drawerOpen} onClose={(): void => setDrawerOpen(false)}>
          <Box sx={{ width: MOBILE_DRAWER_WIDTH }}>{list}</Box>
        </Drawer>
      ) : (
        <Stack
          sx={{
            width: SIDEBAR_WIDTH,
            flexShrink: 0,
            borderRight: 1,
            borderColor: "divider",
            height: "100%",
          }}
        >
          {sidebarHeader}
          <Box sx={{ flex: 1, minHeight: 0 }}>{list}</Box>
        </Stack>
      )}

      <Stack sx={{ flex: 1, minWidth: 0 }}>
        <AppBar position="static" color="default" elevation={0}>
          <Toolbar
            variant="dense"
            sx={{ borderBottom: 1, borderColor: "divider" }}
          >
            {mobile && (
              <IconButton edge="start" onClick={(): void => setDrawerOpen(true)} sx={{ mr: 1 }}>
                <MenuIcon />
              </IconButton>
            )}
            <Typography variant="subtitle1" noWrap sx={{ flex: 1, minWidth: 0 }}>
              {active
                ? mobile
                  ? `🤠 ${active.provider}`
                  : active.provider
                : mobile
                  ? "🤠 cowboy"
                  : ""}
            </Typography>
            {active && (
              <Chip
                size="small"
                label={active.status}
                sx={{ mr: 1, color: statusColor(active.status) }}
                variant="outlined"
              />
            )}
            {!connected && <Chip size="small" color="error" label="offline" sx={{ mr: 1 }} />}
            <IconButton
              onClick={(): void => setSettingsOpen(true)}
              aria-label="settings"
              title="Settings"
            >
              <SettingsIcon />
            </IconButton>
          </Toolbar>
        </AppBar>

        {active ? (
          <>
            <Transcript
              sessionId={active.id}
              timeline={timelines.get(active.id) ?? []}
              status={active.status}
            />
            <Composer sessionId={active.id} status={active.status} />
          </>
        ) : (
          // Empty state: relative parent + absolutely-positioned content
          // centered to the geometric middle of the *whole* right pane
          // (including the AppBar area), so it reads as the viewport
          // center even on tall screens. flex-centering would push it
          // ~24px down (half the AppBar height).
          <Box sx={{ flex: 1, position: "relative" }}>
            <Stack
              spacing={2}
              alignItems="center"
              sx={{
                position: "absolute",
                top: "50%",
                left: "50%",
                transform: "translate(-50%, -50%)",
                p: 3,
                width: "max-content",
                maxWidth: "calc(100% - 48px)",
              }}
            >
              <Typography color="text.secondary">No session selected.</Typography>
              <Button variant="contained" startIcon={<Add />} onClick={(): void => setDialogOpen(true)}>
                New session
              </Button>
            </Stack>
          </Box>
        )}
      </Stack>

      <NewSessionDialog open={dialogOpen} onClose={(): void => setDialogOpen(false)} />
      <SettingsShell
        open={settingsOpen}
        bottomSheet={bottomSheet}
        onClose={(): void => setSettingsOpen(false)}
        themeMode={themeMode}
        onSetThemeMode={onSetThemeMode}
      />
    </Box>
  );
}

// Responsive settings container. Desktop / landscape iPad → centred Dialog.
// Mobile / portrait iPad → bottom-anchored Drawer with iOS-style drag
// handle + rounded top corners (the "bottom sheet" idiom). The content is
// the same in both shells.
function SettingsShell({
  open,
  bottomSheet,
  onClose,
  themeMode,
  onSetThemeMode,
}: {
  open: boolean;
  bottomSheet: boolean;
  onClose: () => void;
  themeMode: ThemeMode;
  onSetThemeMode: (m: ThemeMode) => void;
}): React.JSX.Element {
  const body = (
    <Settings themeMode={themeMode} onSetThemeMode={onSetThemeMode} onClose={onClose} />
  );
  if (bottomSheet) {
    return (
      <Drawer
        anchor="bottom"
        open={open}
        onClose={onClose}
        slotProps={{
          paper: {
            sx: {
              borderTopLeftRadius: 16,
              borderTopRightRadius: 16,
              maxHeight: "85vh",
              pb: "env(safe-area-inset-bottom)",
            },
          },
        }}
      >
        {/* Drag handle bar — purely visual; tapping outside still closes. */}
        <Box
          sx={{
            width: 36,
            height: 4,
            borderRadius: 2,
            bgcolor: "action.disabledBackground",
            mx: "auto",
            mt: 1,
            mb: 0.5,
          }}
        />
        {body}
      </Drawer>
    );
  }
  return (
    <Dialog open={open} onClose={onClose} fullWidth maxWidth="xs">
      {body}
    </Dialog>
  );
}
