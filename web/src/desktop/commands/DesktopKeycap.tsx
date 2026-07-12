import { alpha, Box, Stack } from "@mui/material";

function displayKey(key: string): string {
  const normalized = key.trim().toLowerCase();
  if (normalized === "mod") return navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";
  if (normalized === "shift") return "⇧";
  if (normalized === "alt") return navigator.platform.toLowerCase().includes("mac") ? "⌥" : "Alt";
  if (normalized === "escape" || normalized === "esc") return "Esc";
  if (normalized === "backspace") return "⌫";
  if (normalized === "space" || normalized === "spc") return "SPC";
  return key.length === 1 ? key.toUpperCase() : key;
}

export function DesktopKeycap({
  keyLabel,
  accent = false,
}: {
  keyLabel: string;
  accent?: boolean;
}): React.JSX.Element {
  return (
    <Box
      component="kbd"
      aria-hidden
      sx={{
        display: "inline-grid",
        placeItems: "center",
        minWidth: keyLabel.length > 2 ? 30 : 24,
        height: 24,
        px: 0.65,
        borderRadius: 0.8,
        border: 1,
        borderColor: (theme) => alpha(theme.palette.primary.main, accent ? 0.58 : 0.28),
        bgcolor: (theme) => alpha(theme.palette.primary.main, accent ? 0.16 : 0.07),
        color: accent ? "primary.main" : "text.secondary",
        boxShadow: (theme) => [
          `0 1px 2px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.3 : 0.12)}`,
          `inset 0 1px 0 ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.06 : 0.65)}`,
        ].join(", "),
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        fontSize: "0.6875rem",
        fontWeight: 750,
        lineHeight: 1,
        whiteSpace: "nowrap",
      }}
    >
      {displayKey(keyLabel)}
    </Box>
  );
}

export function DesktopShortcut({ shortcut }: { shortcut: string }): React.JSX.Element {
  const keys = shortcut.split("+").filter(Boolean);
  return (
    <Stack direction="row" spacing={0.35} alignItems="center" aria-label={shortcut}>
      {keys.map((key, index) => (
        <DesktopKeycap key={`${key}-${String(index)}`} keyLabel={key} />
      ))}
    </Stack>
  );
}
