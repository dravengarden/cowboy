import { Box, Chip } from "@mui/material";

export function toolStatusLabel(status: string): string {
  if (status === "completed") return "Done";
  if (status === "in_progress") return "Doing";
  // Other ACP states (pending, failed, interrupted) keep their wording but use
  // the same sentence case, so a raw lowercase value never reads as a
  // smaller, different badge beside Done/Doing.
  const words = status.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// A one-word state badge needs less than MUI's 8px small-chip label inset;
// the tighter inset keeps the title room in a narrow phone row.
//
// The theme's rem small-chip height shrinks with the app font scale (1.5rem is
// about 15px at the 65% default), which left the label no vertical inset. The
// badge keeps MUI's original 24px floor, but its slot reserves only the rem
// height, so the taller badge sits in the row's existing padding instead of
// making every tool card taller.
export function ToolStatusChip(
  { status }: { status: string },
): React.JSX.Element {
  return (
    <Box
      sx={{
        height: "1.5rem",
        flexShrink: 0,
        display: "flex",
        alignItems: "center",
      }}
    >
      <Chip
        size="small"
        color={toolColor(status)}
        label={toolStatusLabel(status)}
        variant="outlined"
        sx={{
          height: "max(24px, 1.5rem)",
          // The theme's rem radius is under half of the 24px floor, which
          // squares the badge; keep MUI's original pill at every scale.
          borderRadius: "999px",
          "& .MuiChip-label": { px: 0.75 },
        }}
      />
    </Box>
  );
}

function toolColor(
  status: string,
): "default" | "success" | "error" | "warning" {
  if (status === "completed") return "success";
  if (status === "failed") return "error";
  if (status === "in_progress" || status === "interrupted") return "warning";
  return "default";
}
