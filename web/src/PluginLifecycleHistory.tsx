import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useId, useState } from "react";
import {
  type LifecycleEntry,
  lifecycleEntryKey,
  type LifecycleHistory,
  loadLifecycleHistory,
} from "./pluginLifecycle.ts";
import type { InstallPhase } from "./pluginInstallation.ts";
import type { UninstallPhase } from "./pluginLifecycle.ts";
import { PRODUCT_SESSION_END_EVENT } from "./productSessionEnd.ts";

const installLabels = {
  prepared: "Prepared",
  syncing_authentication: "Synchronizing authentication before installation",
  installing: "Dispatched; awaiting acknowledgement",
  machine_acknowledged: "Machine acknowledged; finalization pending",
  completed: "Attempt finished (historical)",
  authentication_pending: "Machine acknowledged; authentication pending",
  aborted: "Stopped before staging",
  needs_attention: "Reconciliation required — do not repeat installation",
} satisfies Record<InstallPhase, string>;
const uninstallLabels = {
  prepared: "Prepared",
  stopping_sessions: "Stopping approved sessions",
  uninstalling: "Dispatched; awaiting acknowledgement",
  machine_uninstalled: "Machine removal acknowledged; finalization pending",
  restoring_machine: "Legacy Machine compensation in progress",
  restoring_sessions: "Legacy session compensation in progress",
  completed: "Attempt finished (historical)",
  compensated: "Legacy compensation recorded; native turns not verified",
  aborted: "Attempt aborted (historical)",
  needs_attention: "Reconciliation required — do not repeat uninstallation",
} satisfies Record<UninstallPhase, string>;

function EntryView({ entry }: { entry: LifecycleEntry }): React.JSX.Element {
  const op = entry.operation;
  const label = entry.kind === "install"
    ? installLabels[entry.operation.phase]
    : uninstallLabels[entry.operation.phase];
  const receipt = entry.kind === "install"
    ? entry.operation.machine_receipt
    : null;
  return (
    <Stack spacing={0.25} sx={{ overflowWrap: "anywhere" }}>
      <Typography variant="body2">
        {entry.kind === "install" ? "Install" : "Uninstall"} ·{" "}
        {op.plugin_version} · {label}
      </Typography>
      <Typography variant="caption">{op.operation_id}</Typography>
      <Typography variant="caption" color="text.secondary">
        {new Date(op.updated_at_ms).toISOString()} · {op.generation_digest}
      </Typography>
      {entry.kind === "install" && (
        <Typography variant="caption" color="text.secondary">
          {op.evidence_schema === 1
            ? "Legacy Service evidence; no durable Machine receipt"
            : receipt
            ? `Durable Machine receipt: ${receipt.state}${
              receipt.state === "applied"
                ? ` · ${receipt.revision}`
                : "phase" in receipt
                ? ` · ${receipt.phase.replaceAll("_", " ")}`
                : ` · ${receipt.reason.replaceAll("_", " ")}`
            }`
            : "No durable Machine receipt saved"}
        </Typography>
      )}
      {entry.kind === "uninstall" && (
        <Typography variant="caption" color="text.secondary">
          {entry.operation.affected_session_count}{" "}
          approved session references. This view does not query the Machine or
          verify restoration.
        </Typography>
      )}
      {op.problem && (
        <Typography variant="caption" color="text.secondary">
          {op.problem.replaceAll("_", " ")}
          {op.attention_from
            ? ` · after ${op.attention_from.replaceAll("_", " ")}`
            : ""}
        </Typography>
      )}
      {entry.kind === "uninstall" && entry.operation.cause && (
        <Typography variant="caption">
          Original cause: {entry.operation.cause.replaceAll("_", " ")}
        </Typography>
      )}
      {entry.kind === "uninstall" && entry.resolution && (
        <Alert severity="info">
          Separate resolution: {entry.resolution.resolution_id} ·{" "}
          {new Date(entry.resolution.resolved_at_ms).toISOString()}. Confirmed
          abort before effects; no Plugin/session mutation or worker
          restoration.
        </Alert>
      )}
    </Stack>
  );
}

/** Both installation clients borrow this core view. Opening/reloading/refreshing
 * performs only one bounded GET; independent recovery is evidence, not a button
 * that recreates an old confirmation or replays its original operation. */
export function PluginLifecycleHistory(
  { machine, plugin }: { machine: string; plugin: string },
): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [revision, setRevision] = useState(0);
  const [loaded, setLoaded] = useState<
    { target: string; history: LifecycleHistory | null; error: string } | null
  >(null);
  const target = JSON.stringify([machine, plugin]);
  const history = loaded?.target === target ? loaded.history : null;
  const error = loaded?.target === target ? loaded.error : "";
  const id = useId();
  useEffect(() => {
    setLoaded(null);
    if (!open) return;
    const controller = new AbortController();
    const end = () => controller.abort();
    globalThis.addEventListener(PRODUCT_SESSION_END_EVENT, end);
    void loadLifecycleHistory(machine, plugin, controller.signal).then(
      (history) => {
        if (!controller.signal.aborted) {
          setLoaded({ target, history, error: "" });
        }
      },
    ).catch(() => {
      if (!controller.signal.aborted) {
        setLoaded({
          target,
          history: null,
          error:
            "Lifecycle evidence unavailable. This does not mean no operation exists; do not retry based on this error.",
        });
      }
    });
    return () => {
      end();
      globalThis.removeEventListener(PRODUCT_SESSION_END_EVENT, end);
    };
  }, [machine, plugin, target, open, revision]);
  return (
    <Stack spacing={1}>
      <Button
        size="small"
        color="inherit"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((value) => !value)}
      >
        {open ? "Hide Plugin operation history" : "Plugin operation history"}
      </Button>
      {open && (
        <Stack id={id} spacing={1} aria-live="polite">
          <Typography variant="caption" color="text.secondary">
            Latest 32 attempts of each kind; independent durable observations,
            not a complete archive, atomic snapshot, current installation status
            or replay authority. Authentication may have synchronized before an
            installation stopped.
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
          {history &&
            (!history.admission.install || !history.admission.uninstall) && (
            <Alert severity="info">
              New {!history.admission.install ? "installations" : ""}
              {!history.admission.install && !history.admission.uninstall
                ? " and "
                : ""}
              {!history.admission.uninstall ? "uninstallations" : ""}{" "}
              are paused by core admission policy.
            </Alert>
          )}
          {history?.requires_reconciliation && (
            <Alert severity="warning">
              This installation slot is protected pending reconciliation.
              Refreshing evidence does not clear it.
            </Alert>
          )}
          {history?.entries.length === 0 && (
            <Typography variant="caption">
              No saved attempts in this bounded history window.
            </Typography>
          )}
          {history?.entries.map((entry) => (
            <EntryView key={lifecycleEntryKey(entry)} entry={entry} />
          ))}
        </Stack>
      )}
    </Stack>
  );
}
