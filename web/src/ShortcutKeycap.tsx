import { alpha, Box, type SxProps, type Theme } from "@mui/material";

export type ShortcutKeycapVariant = "default" | "global" | "context" | "modal";

export function displayShortcutKey(key: string): string {
  const normalized = key.trim().toLowerCase();
  if (normalized === "mod") {
    return navigator.platform.toLowerCase().includes("mac") ? "⌘" : "Ctrl";
  }
  if (normalized === "shift") return "⇧";
  if (normalized === "alt") {
    return navigator.platform.toLowerCase().includes("mac") ? "⌥" : "Alt";
  }
  if (normalized === "escape" || normalized === "esc") return "Esc";
  if (normalized === "enter" || normalized === "return" || normalized === "↵") {
    return "↵";
  }
  if (normalized === "backspace") return "⌫";
  if (normalized === "space" || normalized === "spc") return "SPC";
  return key.length === 1 ? key.toUpperCase() : key;
}

/**
 * One visual grammar for every Desktop shortcut hint.
 *
 * - global: quiet, persistent labels beside globally reachable targets;
 * - context: compact floating labels that appear with a focused region;
 * - modal: explicit Enter/Escape affordances inside dialog actions;
 * - default: command palette / shortcut guide keycaps.
 *
 * Product-surface gating belongs to callers such as `Kbd`; this primitive only
 * owns the shared visual grammar.
 */
export function ShortcutKeycap({
  keyLabel,
  variant = "default",
  accent = false,
  sx,
}: {
  keyLabel: string;
  variant?: ShortcutKeycapVariant;
  accent?: boolean;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  const rendered = displayShortcutKey(keyLabel);
  const quiet = variant === "global";
  const compact = quiet || variant === "context" || variant === "modal";
  return (
    <Box
      component="kbd"
      aria-hidden
      sx={[
        {
          display: "inline-grid",
          placeItems: "center",
          minWidth: compact
            ? (rendered.length > 2 ? 24 : 18)
            : (rendered.length > 2 ? 28 : 20),
          height: compact ? 18 : 20,
          px: compact ? 0.35 : 0.5,
          borderRadius: 0.75,
          border: 1,
          borderColor: (theme) => {
            if (variant === "context") {
              return alpha(
                theme.palette.primary.main,
                0.34,
              );
            }
            if (accent) {
              return alpha(
                theme.palette.primary.main,
                quiet ? 0.28 : 0.42,
              );
            }
            return alpha(theme.palette.divider, quiet ? 0.52 : 0.78);
          },
          bgcolor: (theme) => {
            if (variant === "context") {
              return alpha(
                theme.palette.background.paper,
                0.9,
              );
            }
            if (accent) {
              return alpha(
                theme.palette.primary.main,
                quiet ? 0.055 : 0.1,
              );
            }
            return alpha(theme.palette.background.paper, quiet ? 0.24 : 0.72);
          },
          color: variant === "context" || accent
            ? "primary.main"
            : (quiet ? "text.disabled" : "text.secondary"),
          boxShadow: quiet ? "none" : (theme) =>
            `0 1px 3px ${
              alpha(
                theme.palette.common.black,
                theme.palette.mode === "dark" ? 0.22 : 0.09,
              )
            }`,
          backdropFilter: quiet ? "none" : "blur(8px)",
          pointerEvents: "none",
          userSelect: "none",
          fontFamily:
            "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
          fontSize: variant === "context" ? "0.5625rem" : "0.625rem",
          fontWeight: 750,
          lineHeight: 1,
          whiteSpace: "nowrap",
        },
        ...(Array.isArray(sx) ? sx : [sx]),
      ]}
    >
      {rendered}
    </Box>
  );
}
