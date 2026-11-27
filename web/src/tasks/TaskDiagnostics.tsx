import { Box, Button, Typography } from "@mui/material";
import { setCallsDockLegacy, type TaskSummary } from "../backgroundTasks";

export function TaskDiagnostics({ task }: { task: TaskSummary }): React.JSX.Element {
  const rows: [string, string][] = [
    ["source", task.diag.source],
    ["entity", task.diag.entity],
    ["execution", task.diag.execution ?? "none (latest observed state)"],
    ["incarnation", task.diag.incarnation ?? "—"],
    ["source_revision", task.diag.sourceRevision ?? "—"],
    ["observed_seq", task.diag.observedSeq === null ? "—" : String(task.diag.observedSeq)],
    ["anchor_seq", task.diag.anchorSeq === null ? "—" : String(task.diag.anchorSeq)],
    ["reconciled_by", task.diag.reconciledBy ?? "—"],
    [
      "reconciled_at",
      task.diag.reconciledAt === null ? "—" : new Date(task.diag.reconciledAt).toLocaleTimeString(),
    ],
  ];
  return (
    <Box
      component="details"
      data-task-diagnostics
      sx={{
        mt: 1.5,
        color: "text.secondary",
        "& summary": { cursor: "pointer", typography: "caption", fontWeight: 650 },
      }}
    >
      <summary>Diagnostics</summary>
      <Box
        component="dl"
        sx={{
          m: 0,
          mt: 0.75,
          display: "grid",
          gridTemplateColumns: "max-content minmax(0, 1fr)",
          columnGap: 1.5,
          rowGap: 0.25,
          typography: "caption",
          fontFamily: "monospace",
        }}
      >
        {rows.map(([label, value]) => (
          <Box key={label} sx={{ display: "contents" }}>
            <Box component="dt">{label}</Box>
            <Box component="dd" data-diagnostic={label} sx={{ m: 0, overflowWrap: "anywhere" }}>
              {value}
            </Box>
          </Box>
        ))}
      </Box>
    </Box>
  );
}

export function LegacyCallsSwitch({ onSwitch }: { onSwitch: () => void }): React.JSX.Element {
  return (
    <Box sx={{ px: 1.5, py: 0.75, borderTop: 1, borderColor: "divider" }}>
      <Button
        size="small"
        color="inherit"
        data-calls-dock-legacy-switch
        onClick={(): void => {
          setCallsDockLegacy(true);
          onSwitch();
        }}
        sx={{ textTransform: "none", color: "text.secondary", fontSize: "0.75rem" }}
      >
        <Typography component="span" variant="caption" color="inherit">
          Use previous Calls panel on this device
        </Typography>
      </Button>
    </Box>
  );
}
