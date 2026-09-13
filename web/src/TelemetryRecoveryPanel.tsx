import { useEffect, useRef, useState } from "react";
import { Alert, Button, Stack, Typography } from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { type BindingOperation, PreviewDeadline } from "./telemetryBinding";
import {
  confirmRecoveryOnce,
  type RecoveryPlan,
  telemetryRecoveryApi,
} from "./telemetryRecovery";

// Mounted under the exact Service operation digest. Closing this view ends
// observation, not a submitted Machine task or the durable Machine audit.
export function TelemetryRecoveryPanel(
  { operation }: { operation: BindingOperation },
): React.JSX.Element {
  const [plan, setPlan] = useState<RecoveryPlan | null>(null);
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

  function begin(timeout: number) {
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

  async function inspect() {
    if (busy || operation.phase !== "needs_attention") return;
    setMessage(null);
    setPlan(null);
    const work = begin(15_000);
    try {
      const next = await telemetryRecoveryApi.plan(
        operation.operation_id,
        work.signal,
      );
      if (work.current()) {
        if (
          next.operation.machine_id !== operation.machine_id ||
          next.operation.operation_digest !== operation.operation_digest
        ) {
          setMessage({
            kind: "warning",
            text:
              "The Service operation changed. Refresh its evidence before creating another preview.",
          });
          return;
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
            "No exact Prepared Machine evidence could be verified. Unknown, historical or unavailable evidence cannot be cleared here. No mutation was sent.",
        });
      }
    } finally {
      work.finish();
    }
  }

  async function confirm() {
    if (
      !plan || !plan.confirmation_available || busy ||
      submitted.current === plan.plan_id || !deadline.current ||
      deadline.current.ended(performance.now(), Date.now())
    ) {
      setExpired(true);
      return;
    }
    const value = plan;
    submitted.current = value.plan_id; // Claim synchronously, not after React renders busy.
    setPlan(null); // Never preserve a submitted confirmation for retry.
    setMessage({
      kind: "info",
      text: "Checking the exact Machine recovery audit…",
    });
    const work = begin(65_000);
    const result = await confirmRecoveryOnce(value, work.signal, () => {
      if (!work.current()) throw new Error("Recovery observer ended");
      const inspection = new AbortController();
      pending.current = inspection;
      return AbortSignal.any([inspection.signal, AbortSignal.timeout(15_000)]);
    });
    if (work.current()) {
      setMessage(
        result
          ? {
            kind: "success",
            text:
              "Machine recovery recorded. Service bookkeeping was not changed. Refresh and separately review the Service resolution; no export was authorized.",
          }
          : {
            kind: "warning",
            text:
              "The outcome is unverified; the request was not resent. Refresh the Service evidence. Recovery query handles are bounded and are lost on Controller restart; absence is not proof of failure.",
          },
      );
    }
    work.finish();
  }

  return (
    <Stack spacing={1} data-telemetry-binding="machine-recovery">
      {message && <Alert severity={message.kind}>{message.text}</Alert>}
      <Button disabled={busy} onClick={() => void inspect()}>
        Review Machine interruption…
      </Button>
      <ConfirmSheet
        open={plan !== null}
        onClose={() => setPlan(null)}
        title="Close an interrupted Machine attempt"
        actions={
          <>
            <Button onClick={() => setPlan(null)}>Close</Button>
            {plan?.confirmation_available && (
              <Button
                variant="contained"
                disabled={busy || expired}
                onClick={() => void confirm()}
              >
                Confirm Machine recovery
              </Button>
            )}
          </>
        }
      >
        {plan && (
          <Stack spacing={1.5}>
            <Typography sx={{ overflowWrap: "anywhere" }}>
              {plan.operation.machine_id} · {plan.operation.operation_id}
            </Typography>
            <Typography variant="body2">
              Machine result: reject the exact Prepared attempt because its
              original authorization ended. Binding revision{" "}
              {plan.machine_head.revision} and policy epoch{" "}
              {plan.machine_head.policy_epoch} stay unchanged.
            </Typography>
            <Typography variant="body2">
              A Prepared query does not prove restart recovery is eligible. The
              Machine must independently verify a validated journal reopen, its
              original connection and the exact current head before admitting
              this action.
            </Typography>
            <Typography variant="body2">
              Service remains unresolved by this action and requires a separate
              fresh confirmation. No binding replay, Plugin installation,
              session restart, credential restoration or export grant. Already
              emitted telemetry cannot be undone.
            </Typography>
            <Typography variant="caption" sx={{ overflowWrap: "anywhere" }}>
              Exact request: {plan.request_digest}
            </Typography>
            <Typography variant="caption">
              The original one-minute preview deadline also bounds confirmation.
              Closing this sheet does not cancel an admitted Machine action.
            </Typography>
            {!plan.confirmation_available && (
              <Alert severity="info">
                Read-only preview. Production Machine recovery admission is
                closed.
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
