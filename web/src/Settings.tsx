// Settings panel. Single source of truth for what's adjustable per-user.
// Today it's just theme; v0 deliberately keeps it sparse so we don't lock
// the shape in before the real settings (composer effort, model, mode, …)
// arrive.
//
// Responsive shell — picked up at the call site, not here:
// - Desktop / landscape iPad: rendered inside a centred `<Dialog>`.
// - Mobile / portrait iPad: rendered inside a bottom-anchored `<Drawer>`
//   with a drag handle and rounded top corners (the iOS / Material bottom
//   sheet idiom).
//
// Settings.tsx just exposes the **content**; the shell wraps it.

import {
  Box,
  Divider,
  IconButton,
  Stack,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import {
  Brightness4,
  Brightness7,
  Close,
  SettingsBrightness,
} from "@mui/icons-material";

export function Settings({
  themeMode,
  onSetThemeMode,
  onClose,
}: {
  themeMode: "system" | "light" | "dark";
  onSetThemeMode: (mode: "system" | "light" | "dark") => void;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Box>
      <Stack
        direction="row"
        alignItems="center"
        sx={{ px: 2, py: 1.5, borderBottom: 1, borderColor: "divider" }}
      >
        <Typography variant="h6" sx={{ flex: 1 }}>
          Settings
        </Typography>
        <IconButton size="small" onClick={onClose} aria-label="close settings">
          <Close fontSize="small" />
        </IconButton>
      </Stack>

      <Stack spacing={3} sx={{ px: 2, py: 2 }}>
        <Stack spacing={1}>
          <Typography variant="overline" color="text.secondary">
            Appearance
          </Typography>
          <ToggleButtonGroup
            exclusive
            value={themeMode}
            onChange={(_e, v): void => {
              if (v) onSetThemeMode(v as "system" | "light" | "dark");
            }}
            size="small"
            fullWidth
          >
            <ToggleButton value="system" aria-label="system theme">
              <SettingsBrightness fontSize="small" sx={{ mr: 0.5 }} />
              System
            </ToggleButton>
            <ToggleButton value="light" aria-label="light theme">
              <Brightness7 fontSize="small" sx={{ mr: 0.5 }} />
              Light
            </ToggleButton>
            <ToggleButton value="dark" aria-label="dark theme">
              <Brightness4 fontSize="small" sx={{ mr: 0.5 }} />
              Dark
            </ToggleButton>
          </ToggleButtonGroup>
        </Stack>

        <Divider />

        <Stack spacing={0.5}>
          <Typography variant="overline" color="text.secondary">
            About
          </Typography>
          <Typography variant="body2" color="text.secondary">
            cowboy v0.1 — multi-agent panel driving Claude Code / Codex over ACP.
          </Typography>
        </Stack>
      </Stack>
    </Box>
  );
}
