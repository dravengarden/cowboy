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
  quiet = false,
}: {
  keyLabel: string;
  accent?: boolean;
  quiet?: boolean;
}): React.JSX.Element {
  const rendered = displayKey(keyLabel);
  return (
    <Box
      component="kbd"
      aria-hidden
      sx={{
        display: "inline-grid",
        placeItems: "center",
        minWidth: quiet ? 24 : (rendered.length > 2 ? 28 : 20),
        height: quiet ? 18 : 20,
        px: quiet ? 0.25 : 0.5,
        borderRadius: 0.7,
        border: 1,
        borderColor: quiet
          ? "transparent"
          : (theme) => accent
          ? alpha(theme.palette.primary.main, 0.4)
          : alpha(theme.palette.divider, 0.72),
        bgcolor: quiet
          ? "transparent"
          : (theme) => accent
          ? alpha(theme.palette.primary.main, 0.1)
          : alpha(theme.palette.background.paper, 0.66),
        color: quiet ? (accent ? "text.secondary" : "text.disabled") :
          (accent ? "primary.main" : "text.secondary"),
        boxShadow: quiet ? "none" : (theme) =>
          `0 1px 2px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.18 : 0.07)}`,
        backdropFilter: quiet ? "none" : "blur(6px)",
        pointerEvents: "none",
        userSelect: "none",
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
        fontSize: "0.625rem",
        fontWeight: 750,
        lineHeight: 1,
        whiteSpace: "nowrap",
      }}
    >
      {rendered}
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
