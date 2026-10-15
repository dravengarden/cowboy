/** Compact Desktop toolbar action: icon, optional label and an inline keycap.
 * Every dimension is root-relative so the whole control follows the global
 * font size; fixed pixel padding or hit boxes left the glyph scaling inside a
 * frame that did not. Shared by the prompt toolbar and the Queue/Draft row
 * editor so both bars read as one control family. */
export const desktopToolbarActionSx = {
  minWidth: 0,
  minHeight: "2rem",
  px: "0.375rem",
  py: "0.125rem",
  gap: "0.25rem",
  borderRadius: 1,
  textTransform: "none",
  whiteSpace: "nowrap",
  fontSize: "0.75rem",
  fontWeight: 500,
  "& .MuiSvgIcon-root": {
    fontSize: "1.125rem",
    width: "1.125rem",
    height: "1.125rem",
  },
} as const;
