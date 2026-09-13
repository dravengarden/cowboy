import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Button, Stack, Typography } from "@mui/material";
import { ConfirmSheet } from "./Sheet";
import { TelemetryRecoveryPanel } from "./TelemetryRecoveryPanel";
import { TelemetryMutationPanel } from "./TelemetryMutationPanel";
import {
  type BindingHead,
  type BindingStatus,
  confirmResolutionOnce,
  PreviewDeadline,
  resolutionLabels,
  type ResolutionPlan,
  telemetryBindingApi,
} from "./telemetryBinding";

function Head({ value }: { value: BindingHead | null }): React.JSX.Element {
  return (
    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
      {value === null
        ? "No binding head recorded on the Service"
        : `Revision ${value.revision} · policy epoch ${value.policy_epoch} · ${
          value.selection
            ? `${value.selection.plugin_id} ${value.selection.plugin_version}`
            : "no backend selected"
        }`}
    </Typography>
  );
}

export function TelemetryBindingPanel(): React.JSX.Element {
  const [status, setStatus] = useState<BindingStatus | null>(null);
  const [plan, setPlan] = useState<ResolutionPlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [expired, setExpired] = useState(false);
  const [message, setMessage] = useState<
    { kind: "info" | "warning" | "success"; text: string } | null
  >(null);
  const sequence = useRef(0);
  const lastSubmitted = useRef<string | null>(null);
  const pending = useRef<AbortController | null>(null);
  const deadline = useRef<PreviewDeadline | null>(null);

  const begin = useCallback((timeout: number) => {
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
  }, []);

  const refresh = useCallback(async () => {
    setPlan(null);
    setMessage(null);
    const work = begin(15_000);
    try {
      const next = await telemetryBindingApi.status(work.signal);
      if (work.current()) setStatus(next);
    } catch {
      if (work.current()) {
        setStatus(null);
        setMessage({
          kind: "warning",
          text:
            "Binding evidence is unavailable. Operator access and a compatible Controller are required; no action was retried.",
        });
      }
    } finally {
      work.finish();
    }
  }, [begin]);

  useEffect(() => {
    void refresh();
    const invalidate = () => {
      ++sequence.current;
      pending.current?.abort();
      deadline.current = null;
      setPlan(null);
      setStatus(null);
      setBusy(false);
    };
    globalThis.addEventListener("cowboy:product-sign-out", invalidate);
    return () => {
      ++sequence.current;
      pending.current?.abort();
      globalThis.removeEventListener("cowboy:product-sign-out", invalidate);
    };
  }, [refresh]);

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

  const inspect = async () => {
    if (status?.journal.state !== "retained") return;
    setMessage(null);
    setPlan(null);
    const operation = status.journal.latest;
    const work = begin(15_000);
    try {
      const next = await telemetryBindingApi.plan(
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
              "The operation changed. Refresh its evidence before opening a new preview.",
          });
          return;
        }
        deadline.current = new PreviewDeadline(
          next.expires_at_ms,
          performance.now(),
          Date.now(),
        );
        setExpired(deadline.current.ended(performance.now(), Date.now()));
        setPlan(next);
      }
    } catch {
      if (work.current()) {
        setMessage({
          kind: "warning",
          text:
            "No exact finite resolution could be verified. Refresh to inspect current evidence. Unknown or unresolved Machine state cannot be cleared here.",
        });
      }
    } finally {
      work.finish();
    }
  };

  const confirm = async () => {
    if (
      !plan || !plan.confirmation_available || busy ||
      lastSubmitted.current === plan.plan_id || !deadline.current ||
      deadline.current.ended(performance.now(), Date.now())
    ) {
      setExpired(true);
      return;
    }
    const submitted = plan;
    lastSubmitted.current = submitted.plan_id; // Claim synchronously, before React renders busy.
    setPlan(null); // Never preserve a submitted confirmation for a retry.
    setMessage({
      kind: "info",
      text: "Checking the Service resolution result…",
    });
    const work = begin(70_000);
    const verified = await confirmResolutionOnce(submitted, work.signal, () => {
      if (!work.current()) throw new Error("Confirmation observer ended");
      const inspection = new AbortController();
      pending.current = inspection;
      return AbortSignal.any([inspection.signal, AbortSignal.timeout(15_000)]);
    });
    if (work.current()) {
      setStatus(null);
      setMessage(
        verified
          ? {
            kind: "success",
            text:
              "Service resolution recorded. No Machine action or telemetry export was authorized. Refresh to read current binding evidence.",
          }
          : {
            kind: "warning",
            text:
              "The outcome is unverified. The request was not resent. Refresh to inspect the saved resolution before creating any new confirmation.",
          },
      );
    }
    work.finish();
  };

  const operation = status?.journal.state === "retained"
    ? status.journal.latest
    : null;
  const inspectable = operation &&
    ["prepared", "dispatching", "needs_attention"].includes(operation.phase);
  return (
    <Stack spacing={1} data-telemetry-binding="service">
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography variant="overline" color="text.secondary">
          Telemetry binding
        </Typography>
        <Button size="small" disabled={busy} onClick={() => void refresh()}>
          Refresh
        </Button>
      </Stack>
      <Typography variant="body2" color="text.secondary">
        Core-owned Service evidence and explicit recovery. Installing a
        telemetry Plugin is separate from enabling export. Local rotating files
        remain independent.
      </Typography>
      {message && <Alert severity={message.kind}>{message.text}</Alert>}
      {busy && (
        <Typography role="status" variant="caption">
          Reading verified evidence…
        </Typography>
      )}
      {status?.resolution_admission === "closed" && (
        <Typography variant="body2" color="text.secondary">
          Service resolution writes are closed. Previews do not enable
          production admission.
        </Typography>
      )}
      {status?.journal.state === "absent" && (
        <Typography variant="body2">
          No managed binding journal. Explicitly configured legacy export may
          still be active; this view does not inspect private destinations or
          credentials.
        </Typography>
      )}
      {status?.journal.state === "retained" && (
        <>
          <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
            {status.journal.latest.machine_id} ·{" "}
            {status.journal.latest.phase.replaceAll("_", " ")} ·{" "}
            {status.journal.latest.operation_id}
          </Typography>
          <Head value={status.journal.current} />
          <Typography variant="caption" color="text.secondary">
            Retained evidence keeps legacy fallback closed; it is not an export
            grant.
          </Typography>
          {status.journal.resolution && (
            <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>
              Recorded resolution:{" "}
              {resolutionLabels[status.journal.resolution.action]} ·{" "}
              {status.journal.resolution.resolution_id}
            </Typography>
          )}
        </>
      )}
      {inspectable && (
        <Button disabled={busy} onClick={() => void inspect()}>
          Review resolution…
        </Button>
      )}
      {operation?.phase === "needs_attention" && !busy && (
        <TelemetryRecoveryPanel
          key={`${operation.operation_id}:${operation.operation_digest}`}
          operation={operation}
        />
      )}
      {status && !busy && !inspectable && (
        <TelemetryMutationPanel
          key={operation?.operation_digest ?? "unmanaged"}
          status={status}
        />
      )}
      <ConfirmSheet
        open={plan !== null}
        onClose={() => setPlan(null)}
        title={plan ? resolutionLabels[plan.action] : "Telemetry resolution"}
        actions={
          <>
            <Button onClick={() => setPlan(null)}>Close</Button>
            {plan?.confirmation_available && (
              <Button
                variant="contained"
                disabled={busy || expired}
                onClick={() => void confirm()}
              >
                Confirm Service resolution
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
              Original change: {plan.operation.change.kind}. Service result:
              {" "}
              {plan.result_phase}.
            </Typography>
            <Head value={plan.result_head} />
            <Typography variant="caption" sx={{ overflowWrap: "anywhere" }}>
              Exact operation: {plan.operation.operation_digest}
            </Typography>
            <Typography variant="body2">
              Only Service bookkeeping changes. No Machine mutation, Plugin
              installation, session restart, credential restoration or export
              authorization. Already emitted telemetry cannot be undone.
            </Typography>
            {plan.action !== "abort_before_dispatch" && (
              <Typography variant="body2">
                Confirmation queries the Machine again. Changed or unresolved
                evidence will be refused.
              </Typography>
            )}
            {!plan.confirmation_available && (
              <Alert severity="info">
                Read-only preview. Production resolution admission is closed.
              </Alert>
            )}
            {expired && (
              <Alert severity="warning">
                Preview expired. Close and explicitly review a new preview.
              </Alert>
            )}
          </Stack>
        )}
      </ConfirmSheet>
    </Stack>
  );
}
