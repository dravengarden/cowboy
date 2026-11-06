import { Box, Chip, type ChipProps } from "@mui/material";

/** Status pills retain vertical breathing room at reduced app font scales. */
export function StatusChip({ size = "small", sx, ...props }: ChipProps): React.JSX.Element {
  const slotHeight = size === "small" ? "1.5rem" : "2rem";
  return (
    <Box sx={{ height: slotHeight, flexShrink: 0, display: "inline-flex", alignItems: "center" }}>
      <Chip
        size={size}
        variant="outlined"
        {...props}
        sx={[
          ...(Array.isArray(sx) ? sx : sx ? [sx] : []),
          {
            height: `max(24px, ${slotHeight})`,
            borderRadius: "999px",
            "& .MuiChip-label": { px: 0.75 },
          },
        ]}
      />
    </Box>
  );
}
