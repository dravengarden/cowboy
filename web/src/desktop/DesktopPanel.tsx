import { Box, Stack, type SxProps, type Theme, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { desktopPanelSx } from "./DesktopEmbeddedControl";

/**
 * A labelled group inside a Desktop modal — the control center's one grouping
 * primitive. Every tab (Settings, Notifications, Providers, Machines, Info,
 * Logs) groups with this so no tab falls back to bare rows separated by
 * dividers, which in dark mode read as an undifferentiated black sheet.
 */
export function DesktopPanel({
  label,
  description,
  actions,
  children,
  sx,
}: {
  label: string;
  description?: string;
  actions?: ReactNode;
  children: ReactNode;
  sx?: SxProps<Theme>;
}): React.JSX.Element {
  return (
    <Box
      sx={[
        { ...desktopPanelSx(), p: 1 },
        ...(Array.isArray(sx) ? sx : sx ? [sx] : []),
      ]}
    >
      <Stack
        direction="row"
        alignItems="flex-start"
        justifyContent="space-between"
        spacing={1}
        sx={{ px: 1.5 }}
      >
        <Box sx={{ minWidth: 0 }}>
          <Typography variant="overline" color="text.secondary">
            {label}
          </Typography>
          {description && (
            <Typography
              variant="caption"
              color="text.secondary"
              sx={{ display: "block", mt: -0.5, lineHeight: 1.4 }}
            >
              {description}
            </Typography>
          )}
        </Box>
        {actions}
      </Stack>
      {children}
    </Box>
  );
}
