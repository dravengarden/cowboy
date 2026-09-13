import { useEffect, useRef, useState } from "react";
import {
  Alert,
  Button,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { type BindingStatus, PreviewDeadline } from "./telemetryBinding";
import {
  type BindingChoices,
  type BindingIntent,
  type BindingPlan,
  confirmBindingOnce,
  telemetryMutationApi,
} from "./telemetryMutation";

// Scoped to the parent's exact observed Service operation (or absence).
// This core confirmation does not install a Plugin or enable background export.
export function TelemetryMutationPanel(
  { status }: { status: BindingStatus },
): React.JSX.Element {
  const [choices, setChoices] = useState<BindingChoices | null>(null);
  const [target, setTarget] = useState("");
  const [plan, setPlan] = useState<BindingPlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [expired, setExpired] = useState(false);
  const [message, setMessage] = useState<
    { kind: "info" | "warning" | "success"; text: string } | null
  >(null);
  const sequence = useRef(0);
  const submitted = useRef<string | null>(null);
  const pending = useRef<AbortController | null>(null);
  const deadline = useRef<PreviewDeadline | null>(null);

  useEffect(() => {
    const invalidate = () => {
      ++sequence.current;
      pending.current?.abort();
      deadline.current = null;
      setPlan(null);
      setChoices(null);
      setBusy(false);
      setMessage(null);
    };
    globalThis.addEventListener("cowboy:product-sign-out", invalidate);
    return () => {
      ++sequence.current;
      pending.current?.abort();
      globalThis.removeEventListener("cowboy:product-sign-out", invalidate);
    };
  }, []);
  useEffect(() => {
    if (!plan) return;
    const check = () =>
      setExpired(
        deadline.current?.ended(performance.now(), Date.now()) ?? true,
      );
    check();
    const timer = setInterval(check, 250);
    return () => clearInterval(timer);
  }, [plan]);

  function begin(timeout = 15_000) {
    const id = ++sequence.current;
    pending.current?.abort();
    const controller = new AbortController();
    pending.current = controller;
    const timer = setTimeout(() => controller.abort(), timeout);
    setBusy(true);
    return {
      signal: controller.signal,
      current: () => sequence.current === id,
      finish: () => {
        clearTimeout(timer);
        if (sequence.current === id) setBusy(false);
      },
    };
  }

  async function load() {
    if (busy) return;
    setChoices(null);
    setTarget("");
    setPlan(null);
    setMessage(null);
    const work = begin();
    try {
      const next = await telemetryMutationApi.choices(work.signal);
      if (work.current()) setChoices(next);
    } catch {
      if (work.current()) {
        setMessage({
          kind: "warning",
          text:
            "Binding choices are unavailable. No installation, binding or export was attempted.",
        });
      }
    } finally {
      work.finish();
    }
  }

  async function inspect(intent: BindingIntent) {
    if (busy) return;
    setPlan(null);
    setMessage(null);
    const work = begin();
    try {
      const next = await telemetryMutationApi.plan(intent, work.signal);
      if (work.current()) {
        const expected = status.journal.state === "retained"
          ? status.journal.current
          : null;
        if (
          JSON.stringify(next.operation.expected) !==
            JSON.stringify(expected) ||
          (status.journal.state === "retained" &&
            next.operation.machine_id !== status.journal.latest.machine_id)
        ) {
          throw new Error("Service evidence changed");
        }
        deadline.current = new PreviewDeadline(
          next.expires_at_ms,
          performance.now(),
          Date.now(),
          60_000,
        );
        setExpired(deadline.current.ended(performance.now(), Date.now()));
        setPlan(next);
      }
    } catch {
      if (work.current()) {
        setMessage({
          kind: "warning",
          text:
            "No exact binding preview was verified. Refresh Service evidence and installed targets; stale installations and unresolved Machine state are refused.",
        });
      }
    } finally {
      work.finish();
    }
  }

  async function confirm() {
    if (
      !plan || !plan.confirmation_available || busy ||
      submitted.current === plan.plan_id ||
      !deadline.current || deadline.current.ended(performance.now(), Date.now())
    ) {
      setExpired(true);
      return;
    }
    const value = plan;
    submitted.current = value.plan_id; // Claim before React renders busy.
    setPlan(null);
    setChoices(null);
    setMessage({ kind: "info", text: "Checking the exact binding operation…" });
    const work = begin(70_000);
    const receipt = await confirmBindingOnce(value, work.signal, () => {
      if (!work.current()) throw new Error("Binding observer ended");
      const inspection = new AbortController();
      pending.current = inspection;
      return AbortSignal.any([inspection.signal, AbortSignal.timeout(15_000)]);
    });
    if (work.current()) {
      const phase = receipt?.operation.phase;
      setMessage(
        phase === "completed"
          ? {
            kind: "success",
            text:
              "Binding operation recorded as completed. This is historical evidence, not a background export grant. Refresh Service evidence before any new action.",
          }
          : phase === "rejected" || phase === "aborted"
          ? {
            kind: "warning",
            text:
              "The binding was not applied. Its journal and legacy-export fence remain retained. Refresh Service evidence before any new action.",
          }
          : {
            kind: "warning",
            text:
              "The outcome is unverified; the request was not resent. Refresh Service evidence and separately review any unresolved operation. A missing receipt is not proof of failure.",
          },
      );
    }
    work.finish();
  }

  const selected = target === "" ? undefined : choices?.targets[Number(target)];
  return (
    <Stack spacing={1} data-telemetry-binding="mutation">
      {message && <Alert severity={message.kind}>{message.text}</Alert>}
      <Button disabled={busy} onClick={() => void load()}>
        Review binding choices…
      </Button>
      {choices && (
        <>
          {!choices.confirmation_available && (
            <Typography variant="body2" color="text.secondary">
              Binding writes are closed. You can inspect exact read-only
              previews.
            </Typography>
          )}
          {choices.targets.length > 0
            ? (
              <>
                <TextField
                  select
                  fullWidth
                  size="small"
                  label="Installed telemetry backend"
                  value={target}
                  disabled={busy}
                  onChange={(event) => {
                    setTarget(event.target.value);
                    setPlan(null);
                  }}
                >
                  <MenuItem value="">Choose an installation</MenuItem>
                  {choices.targets.map((entry, index) => (
                    <MenuItem
                      key={`${entry.machine_id}:${entry.installation.installation_revision}`}
                      value={String(index)}
                    >
                      {entry.machine_id} · {entry.installation.plugin_id}{" "}
                      {entry.installation.plugin_version}
                    </MenuItem>
                  ))}
                </TextField>
                <Button
                  disabled={busy || !selected}
                  onClick={() => {
                    if (selected) {
                      void inspect({ action: "select", target: selected });
                    }
                  }}
                >
                  Preview selection…
                </Button>
              </>
            )
            : (
              <Typography variant="body2">
                No eligible installed telemetry backend on a compatible
                connected Machine.
              </Typography>
            )}
          {choices.revoke_available && (
            <Button
              disabled={busy}
              onClick={() => void inspect({ action: "revoke" })}
            >
              Preview revocation…
            </Button>
          )}
          {choices.restore_operation_id && (
            <Button
              disabled={busy}
              onClick={() => {
                if (choices.restore_operation_id) {
                  void inspect({
                    action: "restore",
                    operation_id: choices.restore_operation_id,
                  });
                }
              }}
            >
              Preview exact restoration…
            </Button>
          )}
        </>
      )}
      <ConfirmSheet
        open={plan !== null}
        onClose={() => setPlan(null)}
        title={plan ? `Review binding ${plan.action}` : "Telemetry binding"}
        actions={
          <>
            <Button onClick={() => setPlan(null)}>Close</Button>
            {plan?.confirmation_available && (
              <Button
                variant="contained"
                disabled={busy || expired}
                onClick={() => void confirm()}
              >
                Confirm binding {plan.action}
              </Button>
            )}
          </>
        }
      >
        {plan && (
          <Stack spacing={1.5}>
            <Typography sx={{ overflowWrap: "anywhere" }}>
              {plan.operation.machine_id} · {plan.plan_id}
            </Typography>
            <Typography variant="body2">
              If applied: revision {plan.result_head.revision} · policy epoch
              {" "}
              {plan.result_head.policy_epoch} · {plan.result_head.selection
                ? `${plan.result_head.selection.plugin_id} ${plan.result_head.selection.plugin_version}`
                : "no backend selected"}.
            </Typography>
            {plan.result_head.selection && (
              <Typography variant="caption" sx={{ overflowWrap: "anywhere" }}>
                Exact installation:{" "}
                {plan.result_head.selection.installation_revision}
                <br />Release: {plan.result_head.selection.generation_digest}
              </Typography>
            )}
            {plan.restores_operation_id && (
              <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
                Restores only the recorded prior selection of{" "}
                {plan.restores_operation_id}; both counters advance. A removed
                or reinstalled target is refused.
              </Typography>
            )}
            <Typography variant="body2">
              Creating the first Service intent or Machine namespace closes
              legacy export admission, even if this operation later aborts or is
              rejected. That fence is not automatically undone.
            </Typography>
            <Typography variant="body2">
              Confirmation rechecks the original connection, exact installation
              and current Operator. Only the Machine can validate its private
              policy. No Plugin installation, credential restoration, session
              restart or background export grant. Already emitted telemetry
              cannot be undone.
            </Typography>
            <Typography variant="caption" sx={{ overflowWrap: "anywhere" }}>
              Exact request: {plan.request_digest}
            </Typography>
            <Typography variant="caption">
              The original one-minute preview bounds confirmation. Closing this
              sheet does not cancel an admitted operation.
            </Typography>
            {!plan.confirmation_available && (
              <Alert severity="info">
                Read-only preview. Production binding admission is closed.
              </Alert>
            )}
            {expired && (
              <Alert severity="warning">
                Preview expired. Close and explicitly review fresh evidence.
              </Alert>
            )}
          </Stack>
        )}
      </ConfirmSheet>
    </Stack>
  );
}
