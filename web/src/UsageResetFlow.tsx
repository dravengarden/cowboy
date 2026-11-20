import { useState } from "react";
import {
  Button,
  DialogContentText,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { NetworkButton } from "./NetworkActionFeedback";
import { acceptedScheduleTime } from "./usageLimits";
import {
  cancelNearestResetSchedule,
  consumeNearestReset,
  ResetOutcomeUnknown,
  scheduleNearestReset,
  type UsageResetTarget,
} from "./usageApi";

export type UsageResetMode = "schedule" | "now";

/** One card's reset confirmation: schedule the nearest reset or use it now,
 * each behind the typed `confirm`. Mobile (ConfirmSheet) and Desktop (Dialog)
 * render the same flow through their own surface. */
export interface UsageResetFlow {
  open: boolean;
  mode: UsageResetMode;
  fireAt: string;
  confirmText: string;
  busy: boolean;
  error: string | null;
  /** The confirmation and, for a schedule, its time are complete. */
  ready: boolean;
  /** Only Service cards schedule; a named card's reset is immediate. */
  scheduling: boolean;
  setFireAt: (value: string) => void;
  setConfirmText: (value: string) => void;
  openDialog: () => void;
  close: () => void;
  changeMode: (value: UsageResetMode | null) => void;
  cancelSchedule: () => Promise<void>;
  submit: () => Promise<void>;
}

function targetKey(target: UsageResetTarget | undefined): string {
  return target === undefined ? "" : target.kind === "service" ? `service:${target.provider}` : `account:${target.accountId}`;
}

export function useUsageResetFlow(
  target: UsageResetTarget | undefined,
  nearestCreditId: string | undefined,
  onUsageChanged: () => Promise<void>,
): UsageResetFlow {
  const scheduling = target?.kind !== "account";
  const initialMode: UsageResetMode = scheduling ? "schedule" : "now";
  const [open, setOpen] = useState(false);
  const [mode, setMode] = useState<UsageResetMode>(initialMode);
  /** The account and grant confirmed at open; a change needs a new confirmation. */
  const [pinned, setPinned] = useState<{ target: string; creditId: string | undefined } | null>(null);
  /** An unknown outcome locks the dialog until it is opened again. */
  const [locked, setLocked] = useState(false);
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scheduleValid = fireAt !== "" && new Date(fireAt).getTime() > Date.now();
  const changed = open && pinned !== null &&
    (pinned.target !== targetKey(target) || (mode === "now" && pinned.creditId !== nearestCreditId));
  const ready = !busy && !locked && !changed && confirmText === "confirm" &&
    (mode !== "schedule" || scheduleValid);
  const clear = (): void => {
    setMode(initialMode);
    setFireAt("");
    setConfirmText("");
    setError(null);
  };
  const run = async (action: () => Promise<void>, failure: string): Promise<void> => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onUsageChanged();
    } catch (cause) {
      if (cause instanceof ResetOutcomeUnknown) {
        setLocked(true);
        void onUsageChanged().catch(() => undefined);
      }
      setError(cause instanceof Error ? cause.message : failure);
    } finally {
      setBusy(false);
    }
  };
  const submit = async (): Promise<void> => {
    if (target === undefined || pinned === null || !ready) return;
    await run(async () => {
      if (mode === "schedule") {
        await scheduleNearestReset(target, new Date(fireAt).getTime(), confirmText);
      } else {
        // Exactly the grant confirmed when the dialog opened.
        await consumeNearestReset(target, pinned.creditId, confirmText);
      }
      setOpen(false);
      setMode(initialMode);
      setConfirmText("");
    }, "Could not use reset");
  };
  useConfirmEnter(open, () => void submit());
  return {
    open,
    mode,
    fireAt,
    confirmText,
    busy,
    error: error ?? (changed ? "The available reset changed. Close and confirm again." : null),
    ready,
    scheduling,
    setFireAt,
    setConfirmText,
    openDialog: () => {
      clear();
      setLocked(false);
      setPinned({ target: targetKey(target), creditId: nearestCreditId });
      setOpen(true);
    },
    close: () => {
      if (busy) return;
      setOpen(false);
      clear();
    },
    changeMode: (value) => {
      if (!value || value === mode || busy || (value === "schedule" && !scheduling)) return;
      setMode(value);
      setConfirmText("");
      setError(null);
    },
    cancelSchedule: async () => {
      if (target === undefined) return;
      await run(() => cancelNearestResetSchedule(target), "Could not cancel schedule");
    },
    submit,
  };
}

/** The dialog body shared by the Mobile sheet and the Desktop dialog. */
export function UsageResetFields({ flow }: { flow: UsageResetFlow }): React.JSX.Element {
  return (
    <>
      <DialogContentText>
        {flow.mode === "schedule"
          ? "At the selected time, Cowboy will use the earliest-expiring reset then available."
          : "Cowboy will immediately use the earliest-expiring available reset. This cannot be undone."}
      </DialogContentText>
      {flow.scheduling && <ToggleButtonGroup
        exclusive
        fullWidth
        value={flow.mode}
        onChange={(_event, value: UsageResetMode | null) => flow.changeMode(value)}
        aria-label="Reset timing"
        sx={{ mt: 2 }}
      >
        <ToggleButton value="schedule">Schedule</ToggleButton>
        <ToggleButton value="now" color="error">Now</ToggleButton>
      </ToggleButtonGroup>}
      {flow.mode === "schedule" && (
        <TextField
          type="datetime-local"
          fullWidth
          label="Run at"
          value={flow.fireAt}
          onChange={(event) => flow.setFireAt(acceptedScheduleTime(event.target.value))}
          slotProps={{ inputLabel: { shrink: true } }}
          helperText="Choose a time at least one minute ahead"
          sx={{ mt: 2 }}
        />
      )}
      <TextField
        autoComplete="off"
        fullWidth
        label="Type confirm to continue"
        value={flow.confirmText}
        onChange={(event) => flow.setConfirmText(event.target.value)}
        sx={{ mt: 2 }}
      />
      {flow.error && (
        <Typography color="error.main" variant="body2" sx={{ mt: 1 }}>
          {flow.error}
        </Typography>
      )}
    </>
  );
}

export function UsageResetActions({ flow }: { flow: UsageResetFlow }): React.JSX.Element {
  return (
    <>
      <Button onClick={flow.close} disabled={flow.busy}>
        Cancel
        <Kbd keys="Esc" availability={flow.busy ? "inactive" : "available"} />
      </Button>
      <NetworkButton
        variant="contained"
        color={flow.mode === "now" ? "error" : "primary"}
        disabled={!flow.ready}
        networkAction={flow.submit}
      >
        {flow.mode === "schedule" ? "Schedule reset" : "Reset now"}
        <Kbd
          keys={`${MOD_LABEL}${ENTER_LABEL}`}
          availability={flow.ready ? "available" : "inactive"}
        />
      </NetworkButton>
    </>
  );
}
