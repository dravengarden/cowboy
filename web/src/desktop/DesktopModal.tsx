import { alpha, Box, Dialog, Divider, Stack, Typography } from "@mui/material";
import type { ReactNode } from "react";
import { Kbd } from "../Kbd";

export function DesktopModal({
  open,
  onClose,
  title,
  description,
  icon,
  children,
  footer,
  width = 920,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  icon?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}): React.JSX.Element {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      fullWidth
      maxWidth={false}
      slotProps={{
        paper: {
          sx: {
            width: `min(${width}px, calc(100vw - 64px))`,
            maxHeight: "min(860px, calc(100vh - 64px))",
            m: 4,
            overflow: "hidden",
            borderRadius: 2.5,
            border: 1,
            borderColor: (theme) => alpha(theme.palette.primary.main, 0.22),
            bgcolor: (theme) => alpha(theme.palette.background.paper, 0.96),
            backgroundImage: (theme) =>
              `linear-gradient(145deg, ${alpha(theme.palette.common.white, theme.palette.mode === "dark" ? 0.035 : 0.42)}, transparent 48%)`,
            backdropFilter: "blur(28px) saturate(145%)",
            boxShadow: (theme) =>
              `0 28px 80px ${alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.48 : 0.22)}`,
          },
        },
        backdrop: {
          sx: {
            bgcolor: (theme) => alpha(theme.palette.common.black, theme.palette.mode === "dark" ? 0.56 : 0.34),
            backdropFilter: "blur(3px)",
          },
        },
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1.25} sx={{ px: 2.25, py: 1.55 }}>
        {icon}
        <Box sx={{ minWidth: 0 }}>
          <Typography variant="subtitle1" fontWeight={780}>{title}</Typography>
          {description && <Typography variant="caption" color="text.secondary">{description}</Typography>}
        </Box>
        <Box sx={{ flex: 1 }} />
        <Stack direction="row" alignItems="center" spacing={0.5} color="text.secondary">
          <Kbd keys="Esc" />
          <Typography variant="caption">Close</Typography>
        </Stack>
      </Stack>
      <Divider />
      <Box sx={{ minHeight: 0, overflow: "auto" }}>{children}</Box>
      {footer && <><Divider />{footer}</>}
    </Dialog>
  );
}
