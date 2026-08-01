import { alpha, type Theme } from "@mui/material";

/** Shared Desktop control boundary: action, state, and shortcut keycap live in
 * one keyboard-first surface. Content layout stays with the owning component. */
export function desktopEmbeddedControlSx({
  active = false,
  open = false,
}: {
  active?: boolean;
  open?: boolean;
} = {}) {
  return {
    border: 1,
    borderStyle: "solid",
    borderColor: (theme: Theme) =>
      alpha(theme.palette.primary.main, open ? 0.68 : active ? 0.5 : 0.3),
    borderRadius: 999,
    bgcolor: (theme: Theme) =>
      alpha(
        theme.palette.background.paper,
        open ? (theme.palette.mode === "dark" ? 0.78 : 0.82) : 0.46,
      ),
    boxShadow: open
      ? (theme: Theme) => `0 0 0 2px ${alpha(theme.palette.primary.main, 0.1)}`
      : "none",
    transition:
      "background-color 120ms ease, border-color 120ms ease, box-shadow 120ms ease",
    "&:hover": {
      borderColor: (theme: Theme) => alpha(theme.palette.primary.main, 0.52),
      bgcolor: (theme: Theme) => alpha(theme.palette.primary.main, 0.06),
    },
    "&.Mui-focusVisible": {
      borderColor: "primary.main",
      boxShadow: (theme: Theme) =>
        `0 0 0 3px ${alpha(theme.palette.primary.main, 0.18)}`,
    },
  };
}
