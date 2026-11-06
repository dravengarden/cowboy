import { KeyboardArrowDown } from "@mui/icons-material";
import { Box, ButtonBase, Typography } from "@mui/material";
import type { ReactNode } from "react";

export function ReviewContextButton({ label, title, subtitle, statusColor, onOpen }: {
  label: string;
  title: string;
  subtitle: ReactNode;
  statusColor: string;
  onOpen: () => void;
}): React.JSX.Element {
  return (
    <ButtonBase data-review-context-button aria-label={label} onClick={onOpen}
      sx={{ flex: 1, minWidth: 0, minHeight: 44, flexDirection: "column", alignItems: "stretch", textAlign: "left" }}>
      <Typography variant="body2" noWrap>{title}</Typography>
      <Box sx={{ display: "flex", alignItems: "center", gap: 0.625, minWidth: 0, color: "text.secondary" }}>
        <Box aria-hidden sx={{ width: 6, height: 6, flex: "0 0 auto", borderRadius: "50%", bgcolor: statusColor }} />
        <Typography variant="caption" noWrap>{subtitle}</Typography>
        <KeyboardArrowDown aria-hidden sx={{ fontSize: "0.9rem", flex: "0 0 auto" }} />
      </Box>
    </ButtonBase>
  );
}
