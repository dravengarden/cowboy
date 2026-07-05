import { useEffect, useState } from "react";
import { Box, Button, Stack, TextField, Typography } from "@mui/material";
import { Sheet } from "./Sheet";
import { SegmentedPill } from "./SegmentedPill";
import { haptic } from "./haptic";
import type { Delivery, DraftSchedule } from "./protocol";
import { fireLabel, fireRel, parseDtLocal, toDtLocal } from "./scheduleTime";

// The schedule picker: a single custom date+time (one native datetime-local
// field), a queue-position choice (tail/head), and a live human confirm line.
// Used both to schedule a fresh draft (from the composer clock) and to
// reschedule/cancel an existing one (its chip). Both positions wait for a running
// turn to finish and respect a paused queue — neither ever bypasses the ⏸ hold.
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
  const [value, setValue] = useState("");
  const [delivery, setDelivery] = useState<Delivery>("back");

  // Re-seed each time the sheet opens so a reopen reflects current state.
  useEffect(() => {
    if (!open) return;
    setValue(initial ? toDtLocal(initial.fire_at_ms) : "");
    setDelivery(initial?.delivery ?? "back");
  }, [open, initial]);

  const now = Date.now();
  const fireAt = parseDtLocal(value);
  const valid = fireAt !== null && fireAt > now;

  const commit = (): void => {
    if (!valid || fireAt === null) return;
    haptic(24);
    onCommit(fireAt, delivery);
    onClose();
  };

  return (
    <Sheet open={open} onClose={onClose} title="定时发送">
      <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
        <TextField
          type="datetime-local"
          size="small"
          fullWidth
          label="发送时间"
          InputLabelProps={{ shrink: true }}
          value={value}
          onChange={(e): void => setValue(e.target.value)}
        />

        {/* Queue position — both wait for the running turn + respect the pause. */}
        <Stack spacing={0.75}>
          <SegmentedPill<Delivery>
            value={delivery}
            onChange={(v): void => {
              haptic(8);
              setDelivery(v);
            }}
            options={[
              { value: "back", label: "队列后方" },
              { value: "front", label: "队列前方" },
            ]}
            sx={{ alignSelf: "flex-start" }}
          />
          <Typography variant="caption" color="text.secondary">
            {delivery === "back"
              ? "到点排到队尾：前面排队的消息发完后再发。"
              : "到点插到队首：先于其他排队消息。"}
            {" 两者都等当前回合结束、遵守队列暂停。"}
          </Typography>
        </Stack>

        {/* Live confirm line */}
        <Typography
          variant="body2"
          sx={{ color: valid ? "info.main" : "text.disabled", fontWeight: 600, minHeight: 22 }}
        >
          {valid && fireAt !== null
            ? `${fireLabel(fireAt)} · ${delivery === "front" ? "队首" : "队尾"} · ${fireRel(fireAt)}`
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
