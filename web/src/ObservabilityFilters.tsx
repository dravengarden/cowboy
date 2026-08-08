import { useEffect, useId, useMemo, useState } from "react";
import {
  Box,
  Button,
  Chip,
  MenuItem,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { AccessTime, Check, Tune } from "@mui/icons-material";
import { Sheet } from "./Sheet";
import { parseDtLocal, toDtLocal } from "./scheduleTime";
import {
  type ObservabilityTimeRange,
  type TimeRangeUnit,
  timeRangeDuration,
  timeRangeLabel,
  validTimeRange,
} from "./observabilityTimeRange";

export interface FilterChipOption<T extends string> {
  value: T;
  label: string;
  color?: "default" | "primary" | "secondary" | "error" | "warning" | "info" | "success";
}

export function MultiSelectChipGroup<T extends string>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: readonly FilterChipOption<T>[];
  value: readonly T[];
  onChange: (value: T[]) => void;
}): React.JSX.Element {
  const selected = new Set(value);
  return (
    <Stack spacing={0.75}>
      <Typography variant="caption" color="text.secondary" fontWeight={700}>{label}</Typography>
      <Stack direction="row" useFlexGap flexWrap="wrap" gap={0.75}>
        <Chip
          label="All"
          size="small"
          variant={value.length === 0 ? "filled" : "outlined"}
          color={value.length === 0 ? "primary" : "default"}
          icon={value.length === 0 ? <Check /> : undefined}
          onClick={() => onChange([])}
        />
        {options.map((option) => {
          const active = selected.has(option.value);
          return (
            <Chip
              key={option.value}
              label={option.label}
              size="small"
              variant={active ? "filled" : "outlined"}
              color={active ? option.color ?? "primary" : "default"}
              icon={active ? <Check /> : undefined}
              onClick={() => onChange(active
                ? value.filter((item) => item !== option.value)
                : [...value, option.value])}
            />
          );
        })}
      </Stack>
    </Stack>
  );
}

export function FilterButton({ count, onClick }: { count: number; onClick: () => void }): React.JSX.Element {
  return (
    <Button
      variant={count > 0 ? "contained" : "outlined"}
      color="inherit"
      size="small"
      startIcon={<Tune />}
      onClick={onClick}
      sx={{ minHeight: 36, borderRadius: 99, flexShrink: 0 }}
    >
      Filters{count > 0 ? ` · ${String(count)}` : ""}
    </Button>
  );
}

export function ActiveFilterChips({
  items,
}: {
  items: Array<{ key: string; label: string; color?: FilterChipOption<string>["color"]; onDelete: () => void }>;
}): React.JSX.Element | null {
  if (items.length === 0) return null;
  return (
    <Stack direction="row" useFlexGap flexWrap="wrap" gap={0.6}>
      {items.map((item) => (
        <Chip
          key={item.key}
          label={item.label}
          color={item.color ?? "default"}
          variant="outlined"
          size="small"
          onDelete={item.onDelete}
        />
      ))}
    </Stack>
  );
}

const QUICK_RANGES: readonly ObservabilityTimeRange[] = [
  { mode: "relative", amount: 1, unit: "hour" },
  { mode: "relative", amount: 2, unit: "hour" },
  { mode: "relative", amount: 6, unit: "hour" },
  { mode: "relative", amount: 24, unit: "hour" },
  { mode: "relative", amount: 7, unit: "day" },
  { mode: "relative", amount: 30, unit: "day" },
];

function sameRange(a: ObservabilityTimeRange, b: ObservabilityTimeRange): boolean {
  if (a.mode !== b.mode) return false;
  if (a.mode === "relative" && b.mode === "relative") {
    return a.amount === b.amount && a.unit === b.unit;
  }
  return a.mode === "absolute" && b.mode === "absolute" && a.fromMs === b.fromMs && a.toMs === b.toMs;
}

export function TimeRangeButton({
  value,
  onChange,
  maxDurationMs,
}: {
  value: ObservabilityTimeRange;
  onChange: (value: ObservabilityTimeRange) => void;
  maxDurationMs: number;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const fieldId = useId();
  const [mode, setMode] = useState<ObservabilityTimeRange["mode"]>(value.mode);
  const [amount, setAmount] = useState(value.mode === "relative" ? value.amount : 24);
  const [unit, setUnit] = useState<TimeRangeUnit>(value.mode === "relative" ? value.unit : "hour");
  const [fromValue, setFromValue] = useState(value.mode === "absolute" ? toDtLocal(value.fromMs) : "");
  const [toValue, setToValue] = useState(value.mode === "absolute" ? toDtLocal(value.toMs) : "");

  useEffect(() => {
    if (!open) return;
    setMode(value.mode);
    if (value.mode === "relative") {
      setAmount(value.amount);
      setUnit(value.unit);
      return;
    }
    setFromValue(toDtLocal(value.fromMs));
    setToValue(toDtLocal(value.toMs));
  }, [open, value]);

  const candidate = useMemo<ObservabilityTimeRange>(() => {
    if (mode === "relative") return { mode, amount, unit };
    return {
      mode,
      fromMs: parseDtLocal(fromValue) ?? 0,
      toMs: parseDtLocal(toValue) ?? 0,
    };
  }, [amount, fromValue, mode, toValue, unit]);
  const valid = validTimeRange(candidate, maxDurationMs);
  const tooLong = timeRangeDuration(candidate) > maxDurationMs;

  return (
    <>
      <Button
        variant="outlined"
        color="inherit"
        size="small"
        startIcon={<AccessTime />}
        onClick={() => setOpen(true)}
        sx={{ minHeight: 36, borderRadius: 99, minWidth: 0, maxWidth: "100%", flex: 1 }}
      >
        <Box component="span" sx={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {timeRangeLabel(value)}
        </Box>
      </Button>
      <Sheet open={open} onClose={() => setOpen(false)} title="Time range" desktopMaxWidth={520}>
        <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
          <Stack spacing={0.75}>
            <Typography variant="caption" color="text.secondary" fontWeight={700}>Quick ranges</Typography>
            <Stack direction="row" useFlexGap flexWrap="wrap" gap={0.75}>
              {QUICK_RANGES.filter((range) => timeRangeDuration(range) <= maxDurationMs).map((range) => (
                <Chip
                  key={timeRangeLabel(range)}
                  label={timeRangeLabel(range)}
                  size="small"
                  color={sameRange(candidate, range) ? "primary" : "default"}
                  variant={sameRange(candidate, range) ? "filled" : "outlined"}
                  onClick={() => {
                    if (range.mode !== "relative") return;
                    setMode("relative");
                    setAmount(range.amount);
                    setUnit(range.unit);
                  }}
                />
              ))}
            </Stack>
          </Stack>
          <ToggleButtonGroup
            exclusive
            fullWidth
            size="small"
            value={mode}
            onChange={(_, next: ObservabilityTimeRange["mode"] | null) => {
              if (next) setMode(next);
            }}
          >
            <ToggleButton value="relative">Last N</ToggleButton>
            <ToggleButton value="absolute">Custom range</ToggleButton>
          </ToggleButtonGroup>
          {mode === "relative"
            ? (
              <Stack direction="row" spacing={1}>
                <TextField
                  id={`${fieldId}-amount`}
                  name={`${fieldId}-amount`}
                  type="number"
                  size="small"
                  fullWidth
                  label="Last"
                  value={amount}
                  inputProps={{ min: 1, max: 10_000, inputMode: "numeric" }}
                  onChange={(event) => setAmount(Math.max(1, Number(event.target.value) || 1))}
                />
                <TextField
                  id={`${fieldId}-unit`}
                  name={`${fieldId}-unit`}
                  select
                  size="small"
                  fullWidth
                  label="Unit"
                  value={unit}
                  onChange={(event) => setUnit(event.target.value as TimeRangeUnit)}
                >
                  <MenuItem value="minute">Minutes</MenuItem>
                  <MenuItem value="hour">Hours</MenuItem>
                  <MenuItem value="day">Days</MenuItem>
                </TextField>
              </Stack>
            )
            : (
              <Stack spacing={1.25}>
                <Box>
                  <Typography variant="caption" color="text.secondary" display="block" sx={{ mb: 0.6 }}>From</Typography>
                  <TextField id={`${fieldId}-from`} name={`${fieldId}-from`} type="datetime-local" size="small" fullWidth value={fromValue} onChange={(event) => setFromValue(event.target.value)} inputProps={{ "aria-label": "From" }} />
                </Box>
                <Box>
                  <Typography variant="caption" color="text.secondary" display="block" sx={{ mb: 0.6 }}>To</Typography>
                  <TextField id={`${fieldId}-to`} name={`${fieldId}-to`} type="datetime-local" size="small" fullWidth value={toValue} onChange={(event) => setToValue(event.target.value)} inputProps={{ "aria-label": "To" }} />
                </Box>
              </Stack>
            )}
          {!valid && (
            <Typography variant="caption" color="error.main">
              {tooLong ? `Range must be ${String(Math.round(maxDurationMs / 86_400_000))} days or less.` : "Choose a valid range ending no later than now."}
            </Typography>
          )}
          <Stack direction="row" spacing={1} justifyContent="flex-end">
            <Button onClick={() => setOpen(false)}>Cancel</Button>
            <Button
              variant="contained"
              disabled={!valid}
              onClick={() => {
                onChange(candidate);
                setOpen(false);
              }}
            >
              Apply
            </Button>
          </Stack>
        </Stack>
      </Sheet>
    </>
  );
}
