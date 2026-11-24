import { useContext, useEffect, useState } from "react";
import { Box, Button, Stack, TextField, Typography } from "@mui/material";
import { Sheet } from "./Sheet";
import { SegmentedTabs } from "./SegmentedTabs";
import { confirmationHaptic, navigationHaptic } from "./haptic";
import { isImeKeyEvent } from "./imeKey";
import { desktopCompositionActive } from "./desktop/commands/keyIntent";
import { Kbd } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import type { Delivery, DraftSchedule } from "./protocol";
import {
  computeSchedulePreset,
  fireLabel,
  fireRel,
  parseDtLocal,
  SCHEDULE_QUICK_PRESET_LABELS,
  type ScheduleQuickPresetId,
  toDtLocal,
} from "./scheduleTime";
import { SurfaceContext } from "./surface/SurfaceProfile";

interface ExactScheduleTime {
  readonly presetId: ScheduleQuickPresetId | null;
  readonly fireAtMs: number;
  readonly inputValue: string;
}

function ScheduleQuickPresets({
  selectedId,
  onSelect,
}: {
  selectedId: ScheduleQuickPresetId | null;
  onSelect: (id: ScheduleQuickPresetId) => void;
}): React.JSX.Element {
  return (
    <Stack spacing={1}>
      <Typography variant="caption" color="text.secondary" sx={{ fontWeight: 600 }}>
        Quick presets
      </Typography>
      <Stack direction="row" spacing={0.75} flexWrap="wrap" useFlexGap>
        {SCHEDULE_QUICK_PRESET_LABELS.map((preset) => (
          <Button
            key={preset.id}
            size="small"
            variant={selectedId === preset.id ? "contained" : "outlined"}
            disableElevation
            data-schedule-preset={preset.id}
            onClick={(): void => {
              navigationHaptic();
              onSelect(preset.id);
            }}
            sx={{ textTransform: "none", minWidth: 0, px: 1.25 }}
          >
            {preset.label}
          </Button>
        ))}
      </Stack>
    </Stack>
  );
}

// The schedule picker: quick presets + custom datetime-local input, queue
// position (back/front), and a live human confirm line. Relative presets are
// computed at activation time and keep their exact UTC epoch-ms until the user
// edits the field, avoiding stale render clocks and DST fall-back ambiguity.
export function ScheduleSheet({
  open,
  onClose,
  initial,
  editing,
  onCommit,
  onUnschedule,
}: {
  open: boolean;
  onClose: () => void;
  /** Pre-fill when rescheduling an existing draft; null for a fresh schedule. */
  initial: DraftSchedule | null;
  /** True when editing an existing scheduled draft (shows the "Cancel schedule" action). */
  editing: boolean;
  onCommit: (fireAtMs: number, delivery: Delivery) => void;
  onUnschedule: () => void;
}): React.JSX.Element {
  const desktop = useContext(SurfaceContext)?.kind === "desktop";
  const [value, setValue] = useState("");
  const [exactTime, setExactTime] = useState<ExactScheduleTime | null>(null);
  const [delivery, setDelivery] = useState<Delivery>("back");

  useEffect(() => {
    if (!open) return;
    if (initial) {
      const inputValue = toDtLocal(initial.fire_at_ms);
      setValue(inputValue);
      setExactTime({ presetId: null, fireAtMs: initial.fire_at_ms, inputValue });
    } else {
      setValue("");
      setExactTime(null);
    }
    setDelivery(initial?.delivery ?? "back");
  }, [open, initial]);

  const now = Date.now();
  const preserved = exactTime && exactTime.inputValue === value
    ? exactTime
    : null;
  const fireAt = preserved ? preserved.fireAtMs : parseDtLocal(value);
  const valid = fireAt !== null && fireAt > now;

  const commit = (): void => {
    if (fireAt === null || fireAt <= Date.now()) return;
    confirmationHaptic();
    onCommit(fireAt, delivery);
    onClose();
  };
  useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      const mod = navigator.platform.toLowerCase().includes("mac")
        ? event.metaKey && !event.ctrlKey
        : event.ctrlKey && !event.metaKey;
      if (!mod || event.repeat || isImeKeyEvent(event) || desktopCompositionActive()) return;
      if (event.key === "Enter" && fireAt !== null && fireAt > Date.now()) {
        event.preventDefault();
        event.stopPropagation();
        commit();
        return;
      }
      if (!editing || event.key !== "Backspace") return;
      event.preventDefault();
      event.stopPropagation();
      confirmationHaptic();
      onUnschedule();
      onClose();
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return (): void =>
      globalThis.removeEventListener("keydown", onKeyDown, true);
  }, [delivery, editing, fireAt, onClose, onCommit, onUnschedule, open]);

  return (
    <Sheet open={open} onClose={onClose} title="Schedule send">
      <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
        <ScheduleQuickPresets
          selectedId={preserved?.presetId ?? null}
          onSelect={(id): void => {
            const fireAtMs = computeSchedulePreset(id, Date.now());
            const inputValue = toDtLocal(fireAtMs);
            setValue(inputValue);
            setExactTime({ presetId: id, fireAtMs, inputValue });
          }}
        />
        <Box>
          <Typography variant="caption" color="text.secondary" sx={{ display: "block", mb: 0.75, fontWeight: 600 }}>
            Send time
          </Typography>
          <TextField
            type="datetime-local"
            size="small"
            fullWidth
            value={value}
            onChange={(e): void => {
              setExactTime(null);
              setValue(e.target.value);
            }}
            inputProps={{ "aria-label": "Send time" }}
          />
        </Box>
        <Stack spacing={0.75}>
          <SegmentedTabs<Delivery>
            semantics="toggle"
            fullWidth={false}
            aria-label="Queue position"
            value={delivery}
            onChange={(v): void => { navigationHaptic(); setDelivery(v); }}
            options={[
              { value: "back", label: "End of queue" },
              { value: "front", label: "Front of queue" },
            ]}
            sx={{ alignSelf: "flex-start" }}
          />
          <Typography variant="caption" color="text.secondary">
            {delivery === "back"
              ? "Appends to the end of the queue after earlier queued messages."
              : "Inserts at the front ahead of other queued messages."}
            {" Both wait for the current turn and respect a paused queue."}
          </Typography>
        </Stack>
        <Typography variant="body2" sx={{ color: valid ? "info.main" : "text.disabled", fontWeight: 600, minHeight: 22 }}>
          {valid && fireAt !== null
            ? `${fireLabel(fireAt)} · ${delivery === "front" ? "Front" : "Back"} · ${fireRel(fireAt)}`
            : "Choose a future time"}
        </Typography>
        <Stack direction="row" spacing={1}>
          {editing && (
            <Button
              color="error"
              variant="outlined"
              onClick={(): void => { confirmationHaptic(); onUnschedule(); onClose(); }}
              sx={{ textTransform: "none", borderRadius: 2 }}
            >
              Cancel schedule
              <Kbd keys={`${MOD_LABEL}⌫`} />
            </Button>
          )}
          <Box sx={{ flex: 1 }} />
          {desktop && (
            <Button color="inherit" onClick={onClose} sx={{ textTransform: "none", borderRadius: 2 }}>
              Cancel
              <Kbd keys="Esc" />
            </Button>
          )}
          <Button
            variant="contained"
            disabled={!valid}
            onClick={commit}
            sx={{ textTransform: "none", borderRadius: 2, minWidth: 96 }}
          >
            {editing ? "Update" : "Schedule"}
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} availability={valid ? "available" : "inactive"} />
          </Button>
        </Stack>
      </Stack>
    </Sheet>
  );
}
