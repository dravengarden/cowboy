import Chip from "@mui/material/Chip";
import WarningAmberOutlined from "@mui/icons-material/WarningAmberOutlined";
import { alpha } from "@mui/material/styles";
import type { SessionMeta } from "./protocol";
import { sessionMachinePresentation } from "./sessionExecution";
import { useReliableTouchTap } from "./useReliableTouchTap";

export function SessionMachineBadge({ session, onInfo, compact = false }: {
  session: SessionMeta;
  onInfo: () => void;
  /** Sized for the subtitle line beside the project, not the title line. */
  compact?: boolean;
}) {
  const machine = sessionMachinePresentation(session);
  const tap = useReliableTouchTap<HTMLDivElement>(onInfo);
  if (!machine.visible) return null;
  return (
    <Chip
      size="small"
      label={machine.label}
      title={machine.description}
      aria-label={machine.description}
      variant="outlined"
      color={machine.unavailable
        ? "warning"
        : machine.remote
        ? "primary"
        : "default"}
      icon={machine.unavailable ? <WarningAmberOutlined /> : undefined}
      onPointerDown={tap.onPointerDown}
      onPointerMove={tap.onPointerMove}
      onPointerUp={tap.onPointerUp}
      onPointerCancel={tap.onPointerCancel}
      onClick={(event) => {
        event.stopPropagation();
        tap.onClick(event);
      }}
      onKeyDown={(event) => event.stopPropagation()}
      sx={{
        height: compact ? "1.25rem" : "1.5rem",
        flexShrink: 0,
        maxWidth: machine.remote ? "12rem" : "8rem",
        fontSize: compact ? "0.6875rem" : "0.75rem",
        boxShadow: "none",
        "&:active, &.Mui-focusVisible": { boxShadow: "none" },
        ...(machine.remote
          ? { bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08) }
          : {}),
        "& .MuiChip-label": {
          px: compact ? "0.5rem" : "0.625rem",
          overflow: "hidden",
          textOverflow: "ellipsis",
        },
      }}
    />
  );
}
