import { alpha, type SxProps, type Theme } from "@mui/material";

/**
 * Quiet, surface-owned scrollbar chrome for Desktop scroll regions.
 *
 * The transparent track prevents the operating-system gutter from reading as
 * a second panel, while the inset thumb remains discoverable without competing
 * with transcript status chips or session-row boundaries.
 */
export const desktopScrollbarSx = {
  scrollbarWidth: "thin",
  scrollbarColor: (theme) =>
    `${alpha(theme.palette.text.primary, 0.2)} transparent`,
  "&::-webkit-scrollbar": {
    width: 8,
    height: 8,
  },
  "&::-webkit-scrollbar-track": {
    backgroundColor: "transparent",
  },
  "&::-webkit-scrollbar-thumb": {
    backgroundColor: (theme) => alpha(theme.palette.text.primary, 0.16),
    backgroundClip: "padding-box",
    border: "2px solid transparent",
    borderRadius: 999,
    minHeight: 32,
  },
  "&:hover::-webkit-scrollbar-thumb": {
    backgroundColor: (theme) => alpha(theme.palette.text.primary, 0.28),
  },
} satisfies SxProps<Theme>;
