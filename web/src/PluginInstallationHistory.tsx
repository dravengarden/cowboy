import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useId, useState } from "react";
import {
  type InstallHistory,
  type InstallPhase,
  loadInstallHistory,
} from "./pluginInstallation.ts";

const labels = {
  prepared: "Prepared",
  syncing_authentication: "Synchronizing authentication before installation",
  installing: "Installation dispatched; awaiting acknowledgement",
  machine_acknowledged: "Machine acknowledged; finalization pending",
  completed: "Attempt finished (historical)",
  authentication_pending: "Machine acknowledged; authentication pending",
  aborted: "Installation stopped before staging",
  needs_attention: "Reconciliation required — do not repeat installation",
} satisfies Record<InstallPhase, string>;

/** Core diagnostics stay outside Plugin-authored surfaces and survive reload
 * through a fresh Service read. There is deliberately no replay action. */
export function PluginInstallationHistory(
  { machine, plugin }: { machine: string; plugin: string },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [revision, setRevision] = useState(0);
  const [loaded, setLoaded] = useState<
    {
      target: string;
      history: InstallHistory | null;
      error: string;
    } | null
  >(null);
  const target = JSON.stringify([machine, plugin]);
  // Never paint a previous Machine's evidence while the new effect is queued.
  const history = loaded?.target === target ? loaded.history : null;
  const error = loaded?.target === target ? loaded.error : "";
  const id = useId();
  useEffect(() => {
    setLoaded(null);
    if (!open) return;
    const controller = new AbortController();
    void loadInstallHistory(machine, plugin, controller.signal).then(
      (value) => {
        if (!controller.signal.aborted) {
          setLoaded({ target, history: value, error: "" });
        }
      },
    ).catch(() => {
      if (!controller.signal.aborted) {
        setLoaded({
          target,
          history: null,
          error:
            "Installation evidence unavailable. This does not mean no operation exists; do not retry based on this error.",
        });
      }
    });
    return () => controller.abort();
  }, [machine, plugin, open, revision, target]);
  return (
    <Stack spacing={1}>
      <Button
        size="small"
        color="inherit"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
      >
        {open ? "Hide installation history" : "Installation history"}
      </Button>
      {open && (
        <Stack id={id} spacing={1} aria-live="polite">
          <Typography variant="caption" color="text.secondary">
            Saved attempts, not current installation status or replay authority.
            Legacy acknowledgements are not durable Machine receipts.
            Authentication may have synchronized before installation stopped.
          </Typography>
          <Button
            size="small"
            onClick={() => setRevision((value) => value + 1)}
          >
            Refresh evidence
          </Button>
          {error && <Alert severity="warning">{error}</Alert>}
          {!history && !error && (
            <Typography variant="caption">Loading evidence…</Typography>
          )}
          {history && !history.admission_enabled && (
            <Alert severity="info">
              New installations are paused for the recovery-reader rollout.
            </Alert>
          )}
          {history?.requires_reconciliation && (
            <Alert severity="warning">
              This installation slot is protected pending reconciliation.
              Refreshing evidence does not clear it.
            </Alert>
          )}
          {history?.operations.length === 0 && (
            <Typography variant="caption">
              No saved installation attempts.
            </Typography>
          )}
          {history?.operations.map((operation) => (
            <Stack
              key={operation.operation_id}
              spacing={0.25}
              sx={{ overflowWrap: "anywhere" }}
            >
              <Typography variant="body2">
                {operation.plugin_version} · {labels[operation.phase]}
              </Typography>
              <Typography variant="caption">
                {operation.operation_id}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {operation.evidence_schema === 1
                  ? "Legacy Service evidence; no durable Machine receipt"
                  : operation.machine_receipt
                  ? `Durable Machine receipt: ${operation.machine_receipt.state}${
                    operation.machine_receipt.state === "applied"
                      ? ` · ${operation.machine_receipt.revision}`
                      : "phase" in operation.machine_receipt
                      ? ` · ${
                        operation.machine_receipt.phase.replaceAll("_", " ")
                      }`
                      : ` · ${
                        operation.machine_receipt.reason.replaceAll("_", " ")
                      }`
                  }`
                  : "No durable Machine receipt saved"}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {new Date(operation.updated_at_ms).toISOString()} ·{" "}
                {operation.generation_digest}
              </Typography>
              {operation.problem && (
                <Typography variant="caption" color="text.secondary">
                  {operation.problem.replaceAll("_", " ")}
                  {operation.attention_from
                    ? ` · after ${
                      operation.attention_from.replaceAll("_", " ")
                    }`
                    : ""}
                </Typography>
              )}
            </Stack>
          ))}
        </Stack>
      )}
    </Stack>
  );
}
