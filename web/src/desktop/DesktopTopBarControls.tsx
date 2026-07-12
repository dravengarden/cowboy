import {
  Box,
  FormControl,
  LinearProgress,
  MenuItem,
  Select,
  Skeleton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { useMemo } from "react";
import { AutoScrollAndStop } from "../Composer";
import type { ConfigOption, Status } from "../protocol";
import { send, useStoreSelector } from "../store";

const OPTION_RANK: Record<string, number> = {
  mode: 0,
  model: 1,
  effort: 2,
  reasoning_effort: 2,
  fast: 3,
  fast_mode: 3,
};
const OPTION_LABELS: Record<string, string> = {
  mode: "Mode",
  model: "Model",
  effort: "Effort",
  reasoning_effort: "Effort",
  fast: "Fast",
  fast_mode: "Fast",
};

function optionWidth(option: ConfigOption): number {
  if (option.id === "model") return 180;
  if (option.id === "mode") return 160;
  if (option.id === "fast" || option.id === "fast_mode") return 100;
  return 120;
}

function optionLabel(option: ConfigOption): string {
  const mapped = OPTION_LABELS[option.id];
  if (mapped) return mapped;
  const name = option.name.toLowerCase();
  if (name.includes("reasoning") && name.includes("effort")) return "Effort";
  if (name.includes("fast")) return "Fast";
  return option.name;
}

export function DesktopTopBarControls({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const optionsBySession = useStoreSelector((snapshot) => snapshot.configOptions);
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const dead = status === "exited" || status === "crashed" || status === "interrupted";
  const options = useMemo(() => {
    const raw = optionsBySession.get(sessionId) ?? [];
    return [...raw].sort((left, right) => {
      const leftIndex = OPTION_RANK[left.id] ?? Number.MAX_SAFE_INTEGER;
      const rightIndex = OPTION_RANK[right.id] ?? Number.MAX_SAFE_INTEGER;
      if (leftIndex === rightIndex) return left.name.localeCompare(right.name);
      return leftIndex - rightIndex;
    });
  }, [optionsBySession, sessionId]);
  const contextPercent = session?.context_size
    ? Math.round(((session.context_used ?? 0) / session.context_size) * 100)
    : null;

  return (
    <Stack
      data-desktop-topbar-controls
      direction="row"
      alignItems="center"
      spacing={0.75}
      sx={{ flex: 1, minWidth: 0, ml: 2, overflow: "hidden" }}
    >
      {options.length === 0 && !dead && (status === "starting" || status === "running")
        ? <Skeleton variant="rounded" width={320} height={34} />
        : options.map((option) => (
          <Tooltip key={option.id} title={option.description ?? option.name}>
            <FormControl
              size="small"
              disabled={dead}
              sx={{ width: optionWidth(option), flexShrink: 1, minWidth: 92 }}
            >
              <Select
                value={String(option.currentValue)}
                inputProps={{ "aria-label": option.name }}
                renderValue={(value): React.ReactNode => {
                  const selected = option.options.find((candidate) => String(candidate.value) === value);
                  return (
                    <Stack direction="row" alignItems="center" spacing={0.5} sx={{ minWidth: 0 }}>
                      <Typography variant="caption" color="text.secondary" noWrap>
                        {optionLabel(option)}
                      </Typography>
                      <Typography variant="caption" fontWeight={700} noWrap>
                        {selected?.name ?? String(option.currentValue)}
                      </Typography>
                    </Stack>
                  );
                }}
                onChange={(event): void => {
                  const selected = option.options.find((candidate) =>
                    String(candidate.value) === event.target.value
                  );
                  if (!selected) return;
                  send({
                    type: "set_config_option",
                    session_id: sessionId,
                    config_id: option.id,
                    value: selected.value,
                  });
                }}
                sx={{
                  height: 34,
                  fontSize: "0.75rem",
                  "& .MuiSelect-select": { display: "flex", alignItems: "center", py: 0.5 },
                }}
              >
                {option.options.map((candidate) => (
                  <MenuItem key={String(candidate.value)} value={String(candidate.value)}>
                    {candidate.name}
                  </MenuItem>
                ))}
              </Select>
            </FormControl>
          </Tooltip>
        ))}

      <Box sx={{ flex: 1, minWidth: 4 }} />

      {contextPercent !== null && (
        <Tooltip title={`${String(session?.context_used ?? 0)} / ${String(session?.context_size ?? 0)} context tokens`}>
          <Box sx={{ width: 96, flexShrink: 0 }}>
            <Stack direction="row" justifyContent="space-between" sx={{ mb: 0.25 }}>
              <Typography variant="caption" color="text.secondary">Context</Typography>
              <Typography variant="caption" fontWeight={700}>{contextPercent}%</Typography>
            </Stack>
            <LinearProgress
              variant="determinate"
              value={Math.min(100, contextPercent)}
              color={contextPercent >= 85 ? "warning" : "primary"}
              sx={{ height: 3, borderRadius: 2 }}
            />
          </Box>
        </Tooltip>
      )}

      <AutoScrollAndStop
        sessionId={sessionId}
        status={status}
        presentation="desktop-toolbar"
      />
    </Stack>
  );
}
