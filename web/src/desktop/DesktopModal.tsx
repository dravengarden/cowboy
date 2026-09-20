import {
  Box,
  Dialog,
  Divider,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { Close } from "@mui/icons-material";
import type { KeyboardEventHandler, ReactNode } from "react";
import { isImeKeyEvent } from "../imeKey";
import {
  desktopModalBackdropSx,
  desktopModalPaperSx,
} from "./DesktopEmbeddedControl";
import {
  DesktopShortcutBar,
  type DesktopShortcutGroup,
} from "./DesktopShortcutBar";

export function DesktopModal({
  open,
  onClose,
  title,
  description,
  icon,
  children,
  footer,
  shortcutGroups,
  onShortcutKeyDown,
  width = 920,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  icon?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  shortcutGroups?: readonly DesktopShortcutGroup[];
  onShortcutKeyDown?: KeyboardEventHandler<HTMLDivElement>;
  width?: number;
}): React.JSX.Element {
  return (
    <Dialog
      open={open}
      onClose={(event, reason): void => {
        if (reason === "escapeKeyDown") {
          const keyEvent = "nativeEvent" in event
            ? event.nativeEvent as KeyboardEvent
            : event as KeyboardEvent;
          if (isImeKeyEvent(keyEvent)) return;
        }
        onClose();
      }}
      fullWidth
      maxWidth={false}
      slotProps={{
        ...(onShortcutKeyDown
          ? { root: { onKeyDown: onShortcutKeyDown } }
          : {}),
        paper: {
          sx: {
            ...desktopModalPaperSx(),
            width: `min(${width}px, calc(100vw - 64px))`,
            maxHeight: "min(860px, calc(100vh - 64px))",
            m: 4,
          },
        },
        backdrop: { sx: desktopModalBackdropSx() },
      }}
    >
      <Stack direction="row" alignItems="center" spacing={1.25} sx={{ px: 2.25, py: 1.55 }}>
        {icon}
        <Box sx={{ minWidth: 0 }}>
          <Typography variant="subtitle1" fontWeight={780}>{title}</Typography>
          {description && <Typography variant="caption" color="text.secondary">{description}</Typography>}
        </Box>
        <Box sx={{ flex: 1 }} />
        <Tooltip title="Close · Esc">
          <IconButton aria-label={`Close ${title}`} onClick={onClose} size="small">
            <Close fontSize="small" />
          </IconButton>
        </Tooltip>
      </Stack>
      <Divider />
      <Box sx={{ minHeight: 0, overflow: "auto" }}>{children}</Box>
      {footer && <><Divider />{footer}</>}
      <DesktopShortcutBar
        groups={shortcutGroups ?? [{ slots: [{ shortcut: "Esc", label: "Close" }] }]}
      />
    </Dialog>
  );
}
