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
  setFireAt: (value: string) => void;
  setConfirmText: (value: string) => void;
  openDialog: () => void;
  close: () => void;
  changeMode: (value: UsageResetMode | null) => void;
  cancelSchedule: () => Promise<void>;
  submit: () => Promise<void>;
}

/** The account and grant the owner confirmed when the dialog opened, and the
 * stable operation that makes a retried request return its original receipt. */
interface ResetIntent {
  target: string;
  creditId: string | undefined;
  operationId: string;
}

function targetKey(target: UsageResetTarget | undefined): string {
  return target === undefined ? "" : target.kind === "service" ? `service:${target.provider}` : `account:${target.accountId}`;
}

function operationId(): string {
  return `web-${crypto.randomUUID()}`;
}

export function useUsageResetFlow(
  target: UsageResetTarget | undefined,
  nearestCreditId: string | undefined,
  onUsageChanged: () => Promise<void>,
  /** Where this card's persisted schedule is cancelled, even when it has no
   * reset target now (its credits disappeared or the account is disabled). */
  scheduleTarget: UsageResetTarget | undefined = target,
): UsageResetFlow {
  const [open, setOpen] = useState(false);
  const [intent, setIntent] = useState<ResetIntent | null>(null);
  /** An unknown outcome locks the dialog until it is opened again. */
  const [locked, setLocked] = useState(false);
  const [mode, setMode] = useState<UsageResetMode>("schedule");
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scheduleValid = fireAt !== "" && new Date(fireAt).getTime() > Date.now();
  // A different account or grant than the one opened needs a new confirmation.
  const changed = open && intent !== null &&
    (intent.target !== targetKey(target) || (mode === "now" && intent.creditId !== nearestCreditId));
  const ready = !busy && !locked && !changed && confirmText === "confirm" &&
    (mode !== "schedule" || scheduleValid);
  const clear = (): void => {
    setMode("schedule");
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
    if (target === undefined || intent === null || !ready) return;
    await run(async () => {
      if (mode === "schedule") {
        await scheduleNearestReset(target, new Date(fireAt).getTime(), confirmText);
      } else {
        // The grant confirmed at open, under the same operation on a retry.
        await consumeNearestReset(target, intent.creditId, confirmText, intent.operationId);
      }
      setOpen(false);
      setMode("schedule");
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
    setFireAt,
    setConfirmText,
    openDialog: () => {
      clear();
      setLocked(false);
      setIntent({ target: targetKey(target), creditId: nearestCreditId, operationId: operationId() });
      setOpen(true);
    },
    close: () => {
      if (busy) return;
      setOpen(false);
      clear();
    },
    changeMode: (value) => {
      if (!value || value === mode || busy) return;
      setMode(value);
      setConfirmText("");
      setError(null);
    },
    cancelSchedule: async () => {
      if (scheduleTarget === undefined) return;
      await run(() => cancelNearestResetSchedule(scheduleTarget), "Could not cancel schedule");
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
      <ToggleButtonGroup
        exclusive
        fullWidth
        value={flow.mode}
        onChange={(_event, value: UsageResetMode | null) => flow.changeMode(value)}
        aria-label="Reset timing"
        sx={{ mt: 2 }}
      >
        <ToggleButton value="schedule">Schedule</ToggleButton>
        <ToggleButton value="now" color="error">Now</ToggleButton>
      </ToggleButtonGroup>
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
