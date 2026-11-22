import { CircularProgress } from "@mui/material";
import { StatusChip } from "../StatusChip";

export function toolStatusLabel(status: string): string {
  if (status === "completed") return "Done";
  if (status === "in_progress") return "Doing";
  // Other ACP states (pending, failed, interrupted) keep their wording but use
  // the same sentence case, so a raw lowercase value never reads as a
  // smaller, different badge beside Done/Doing.
  const words = status.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function ToolStatusChip({ status }: { status: string }): React.JSX.Element {
  // A running call uses the accent tone and a hairline spinner, the same
  // language as live managed calls and in-progress todos. Amber is reserved
  // for states that need attention (interrupted); next to red failures an
  // amber "Doing" read as an error.
  const running = status === "in_progress";
  return (
    <StatusChip
      color={toolColor(status)}
      label={toolStatusLabel(status)}
      icon={running ? <CircularProgress size="0.75rem" thickness={5} color="inherit" /> : undefined}
      sx={running ? { "& .MuiChip-icon": { ml: 0.75, mr: -0.25 } } : undefined}
    />
  );
}

function toolColor(
  status: string,
): "default" | "primary" | "success" | "error" | "warning" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "in_progress") return "primary";
  if (status === "interrupted") return "warning";
  return "default";
}
