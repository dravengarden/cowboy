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
import { Add, Brightness4, Circle, Menu as MenuIcon } from "@mui/icons-material";
import { Composer } from "./Composer";
import { Transcript } from "./Transcript";
import { PROVIDERS, type SessionMeta, type Status } from "./protocol";
import { send, useStore } from "./store";

const DRAWER_WIDTH = 280;

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
          <ListItemButton key={s.id} selected={s.id === activeId} onClick={(): void => onPick(s.id)}>
            <Circle sx={{ fontSize: 10, mr: 1, color: statusColor(s.status) }} />
            <ListItemText
              primary={s.provider}
              secondary={s.cwd}
              slotProps={{
                primary: { noWrap: true },
                secondary: { noWrap: true, variant: "caption" },
              }}
            />
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

function NewSessionDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}): React.JSX.Element {
  const [provider, setProvider] = useState<string>(PROVIDERS[0]);
  const [cwd, setCwd] = useState("");
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
            label="Working directory (optional)"
            placeholder="relative to workspace root"
            value={cwd}
            onChange={(e): void => setCwd(e.target.value)}
          />
          <Button
            variant="contained"
            onClick={(): void => {
              send({ type: "new_session", provider, ...(cwd.trim() ? { cwd: cwd.trim() } : {}) });
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
  onToggleTheme,
}: {
  themeMode: string;
  onToggleTheme: () => void;
}): React.JSX.Element {
  const { connected, sessions, timelines } = useStore();
  const theme = useTheme();
  const mobile = useMediaQuery(theme.breakpoints.down("sm"));
  const [activeId, setActiveId] = useState<string | null>(null);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [dialogOpen, setDialogOpen] = useState(false);

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

  return (
    <Box sx={{ display: "flex", height: "100%" }}>
      {mobile ? (
        <Drawer open={drawerOpen} onClose={(): void => setDrawerOpen(false)}>
          <Box sx={{ width: DRAWER_WIDTH }}>{list}</Box>
        </Drawer>
      ) : (
        <Box
          sx={{ width: DRAWER_WIDTH, flexShrink: 0, borderRight: 1, borderColor: "divider" }}
        >
          {list}
        </Box>
      )}

      <Stack sx={{ flex: 1, minWidth: 0 }}>
        <AppBar position="static" color="default" elevation={1}>
          <Toolbar variant="dense">
            {mobile && (
              <IconButton edge="start" onClick={(): void => setDrawerOpen(true)} sx={{ mr: 1 }}>
                <MenuIcon />
              </IconButton>
            )}
            <Typography variant="subtitle1" noWrap sx={{ flex: 1, minWidth: 0 }}>
              {active ? `🤠 ${active.provider}` : "🤠 cowboy"}
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
            <IconButton onClick={onToggleTheme} aria-label="theme" title={themeMode}>
              <Brightness4 />
            </IconButton>
          </Toolbar>
        </AppBar>

        {active ? (
          <>
            <Transcript sessionId={active.id} timeline={timelines.get(active.id) ?? []} />
            <Composer sessionId={active.id} status={active.status} />
          </>
        ) : (
          <Box
            sx={{
              flex: 1,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              p: 3,
            }}
          >
            <Stack spacing={2} alignItems="center">
              <Typography color="text.secondary">No session selected.</Typography>
              <Button variant="contained" startIcon={<Add />} onClick={(): void => setDialogOpen(true)}>
                New session
              </Button>
            </Stack>
          </Box>
        )}
      </Stack>

      <NewSessionDialog open={dialogOpen} onClose={(): void => setDialogOpen(false)} />
    </Box>
  );
}
