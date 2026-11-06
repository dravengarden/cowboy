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
  return <StatusChip color={toolColor(status)} label={toolStatusLabel(status)} />;
}

function toolColor(
  status: string,
): "default" | "success" | "error" | "warning" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "in_progress" || status === "interrupted") return "warning";
  return "default";
}
