import { useEffect, useState } from "react";
import { Box, Button, Stack, TextField, Typography } from "@mui/material";
import { Schedule } from "@mui/icons-material";
import { Sheet } from "./Sheet";
import { SegmentedPill } from "./SegmentedPill";
import { haptic } from "./haptic";
import type { Delivery, DraftSchedule } from "./protocol";
import {
  combineDateTime,
  fireLabel,
  fireRel,
  quickPresets,
  splitDateTime,
} from "./scheduleTime";

// The schedule picker: quick presets + a custom date/time, a queue/now delivery
// switch, and a live human confirm line. Used both to schedule a fresh draft
// (from the composer clock) and to reschedule/cancel an existing one (its chip).
// Mobile-first: lives inside cowboy's frosted `Sheet` (content-sized, ≤75dvh).
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
  /** True when editing an existing scheduled draft (shows the "取消定时" action). */
  editing: boolean;
  onCommit: (fireAtMs: number, delivery: Delivery) => void;
  onUnschedule: () => void;
}): React.JSX.Element {
  const [fireAt, setFireAt] = useState<number | null>(null);
  const [delivery, setDelivery] = useState<Delivery>("queue");
  const [custom, setCustom] = useState(false);
  const [date, setDate] = useState("");
  const [time, setTime] = useState("");

  // Re-seed each time the sheet opens so a reopen reflects current state.
  useEffect(() => {
    if (!open) return;
    setFireAt(initial?.fire_at_ms ?? null);
    setDelivery(initial?.delivery ?? "queue");
    if (initial) {
      const { date: d, time: t } = splitDateTime(initial.fire_at_ms);
      setDate(d);
      setTime(t);
      setCustom(true);
    } else {
      setCustom(false);
      setDate("");
      setTime("");
    }
  }, [open, initial]);

  const now = Date.now();
  const presets = quickPresets(now).filter((p) => p.ms > now);
  const valid = fireAt !== null && fireAt > now;

  const pickCustom = (d: string, t: string): void => {
    setDate(d);
    setTime(t);
    setFireAt(combineDateTime(d, t));
  };

  const commit = (): void => {
    if (!valid || fireAt === null) return;
    haptic(24);
    onCommit(fireAt, delivery);
    onClose();
  };

  return (
    <Sheet open={open} onClose={onClose} title="定时发送">
      <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
        {/* Quick presets */}
        <Box sx={{ display: "flex", flexWrap: "wrap", gap: 1 }}>
          {presets.map((p) => (
            <Button
              key={p.key}
              size="small"
              variant={fireAt === p.ms && !custom ? "contained" : "outlined"}
              onClick={(): void => {
                haptic(8);
                setCustom(false);
                setFireAt(p.ms);
              }}
              sx={{ textTransform: "none", borderRadius: 999, minHeight: 36 }}
            >
              {p.label}
            </Button>
          ))}
          <Button
            size="small"
            variant={custom ? "contained" : "outlined"}
            startIcon={<Schedule sx={{ fontSize: 18 }} />}
            onClick={(): void => {
              haptic(8);
              setCustom(true);
              if (fireAt !== null && !date) {
                const s = splitDateTime(fireAt);
                setDate(s.date);
                setTime(s.time);
              }
            }}
            sx={{ textTransform: "none", borderRadius: 999, minHeight: 36 }}
          >
            自定义
          </Button>
        </Box>

        {/* Custom date + time (native, dependency-free) */}
        {custom && (
          <Stack direction="row" spacing={1}>
            <TextField
              type="date"
              size="small"
              fullWidth
              value={date}
              onChange={(e): void => pickCustom(e.target.value, time)}
            />
            <TextField
              type="time"
              size="small"
              fullWidth
              value={time}
              onChange={(e): void => pickCustom(date, e.target.value)}
            />
          </Stack>
        )}

        {/* Delivery mode */}
        <Stack spacing={0.75}>
          <SegmentedPill<Delivery>
            value={delivery}
            onChange={(v): void => {
              haptic(8);
              setDelivery(v);
            }}
            options={[
              { value: "queue", label: "排队" },
              { value: "now", label: "立即" },
            ]}
            sx={{ alignSelf: "flex-start" }}
          />
          <Typography variant="caption" color="text.secondary">
            {delivery === "queue"
              ? "到点排入队列：空闲即发，忙则等当前回合结束。"
              : "到点插到队首、绕过暂停尽快执行（不打断进行中的回合）。"}
          </Typography>
        </Stack>

        {/* Live confirm line */}
        <Typography
          variant="body2"
          sx={{ color: valid ? "info.main" : "text.disabled", fontWeight: 600, minHeight: 22 }}
        >
          {valid && fireAt !== null
            ? `${fireLabel(fireAt)} · ${delivery === "queue" ? "排队" : "立即"} · ${fireRel(fireAt)}`
            : "选一个未来的时间"}
        </Typography>

        {/* Actions */}
        <Stack direction="row" spacing={1}>
          {editing && (
            <Button
              color="error"
              variant="outlined"
              onClick={(): void => {
                haptic(24);
                onUnschedule();
                onClose();
              }}
              sx={{ textTransform: "none", borderRadius: 2 }}
            >
              取消定时
            </Button>
          )}
          <Box sx={{ flex: 1 }} />
          <Button
            variant="contained"
            disabled={!valid}
            onClick={commit}
            sx={{ textTransform: "none", borderRadius: 2, minWidth: 96 }}
          >
            {editing ? "更新" : "定时发送"}
          </Button>
        </Stack>
      </Stack>
    </Sheet>
  );
}
