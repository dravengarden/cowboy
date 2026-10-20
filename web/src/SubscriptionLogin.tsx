import { Alert, Button, CircularProgress, MenuItem, Stack, TextField, Typography } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { ConfirmSheet } from "./Sheet";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { useStoreSelector } from "./store";
import { SubscriptionAuthorization } from "./SubscriptionAuthorization";
import { canReplaceLogin, loginCompleted, loginFailure, loginLink, loginOptions, subscriptionLogin, type LoginOptions, type LoginScope, type LoginView } from "./subscriptionLoginController";

export function SubscriptionLogin({ scope, label, disabled, request, status, onChange }: { scope: LoginScope; label: string; disabled: boolean; request?: string; status?: string; onChange?: () => void }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const close = () => { setOpen(false); onChange?.(); };
  return <>
    <Button disabled={disabled} onClick={() => setOpen(true)}>{request ? status === "completed" ? "Details" : "Continue sign-in" : "Sign in…"}</Button>
    <ConfirmSheet open={open} onClose={close} title={`${label} · Sign in`} wide>
      {open && <SubscriptionLoginForm scope={scope} label={label} onClose={close} {...(request ? { request } : {})} />}
    </ConfirmSheet>
  </>;
}

interface LoginFormProps { scope: LoginScope; label: string; onClose: () => void; request?: string; onStarted?: (request: string) => void }
export function SubscriptionLoginForm(props: LoginFormProps): React.JSX.Element {
  const { scope, request } = props;
  // A changed account/request is a different form, including its async owner,
  // selected Machine, authorization code and in-flight browser callbacks.
  return <ScopedSubscriptionLoginForm key={JSON.stringify([scope.owner, scope.dataset, scope.provider, scope.account, request])} {...props} />;
}

function ScopedSubscriptionLoginForm({ scope, label, onClose, request, onStarted }: LoginFormProps): React.JSX.Element {
  const machines = useStoreSelector((state) => state.machines);
  const [machine, setMachine] = useState("");
  const [options, setOptions] = useState<LoginOptions>();
  const [method, setMethod] = useState("");
  const [error, setError] = useState("");
  const [view, setView] = useState<LoginView>({ events: [], busy: false, error: "" });
  const [newSignIn, setNewSignIn] = useState(false);
  const [readinessRevision, setReadinessRevision] = useState(0);
  const reference = newSignIn ? undefined : request;
  const knownRequest = view.pending?.request ?? view.observation?.request_id;
  const requestMismatch = Boolean(reference && view.pending && (view.referenceMismatch || (knownRequest && knownRequest !== reference)));
  const alive = useRef(false);
  const pollingUntil = useRef(Date.now() + 5 * 60_000);
  const [checksPaused, setChecksPaused] = useState(false);
  const owner = useMemo(() => subscriptionLogin(scope, {
    fetch: (path, init) => fetch(path, { ...init, signal: AbortSignal.any([productSessionSignal(), AbortSignal.timeout(30_000)]) }),
    storage: { getItem: (key) => sessionStorage.getItem(key), setItem: (key, value) => sessionStorage.setItem(key, value), removeItem: (key) => sessionStorage.removeItem(key) },
    current: () => alive.current && !productSessionSignal().aborted && productSyncPrincipal() === scope.owner,
    changed: setView, operation: () => crypto.randomUUID(),
  }), [scope.owner, scope.dataset, scope.account, scope.provider]);
  useEffect(() => {
    alive.current = true;
    owner.restore();
    if (request) void owner.resume(request);
    else if (owner.snapshot.pending) void owner.check();
    if (owner.snapshot.pending) setMachine(owner.snapshot.pending.machine);
    return () => { alive.current = false; };
  }, [owner, request]);
  useEffect(() => {
    setOptions(undefined); setMethod(""); setError("");
    if (!machine || reference) return;
    const abort = new AbortController();
    void (async () => {
      try {
        const response = await fetch(`/api/provider-accounts/${scope.account}/login?machine_id=${encodeURIComponent(machine)}`, {
          cache: "no-store", signal: AbortSignal.any([abort.signal, productSessionSignal(), AbortSignal.timeout(30_000)]),
        });
        if (!response.ok) throw new Error("Could not load login readiness.");
        const next = loginOptions(await response.json(), scope, machine);
        if (abort.signal.aborted || !alive.current || productSyncPrincipal() !== scope.owner) return;
        setOptions(next); setMethod(next.methods.find((entry) => entry.available)?.id ?? "");
      } catch { if (!abort.signal.aborted && alive.current) setError("Could not load login readiness. Select the Machine again to retry."); }
    })();
    return () => abort.abort();
  }, [machine, reference, readinessRevision, scope.account, scope.owner, scope.provider]);
  useEffect(() => {
    // Also advance the handoff after a lost start reply is recovered through
    // Check status or Retry same sign-in, so reload retains the new request.
    if (newSignIn && view.observation?.request_id) onStarted?.(view.observation.request_id);
  }, [newSignIn, view.observation?.request_id, onStarted]);
  const resetChecks = () => { pollingUntil.current = Date.now() + 5 * 60_000; setChecksPaused(false); };
  const checkStatus = () => { resetChecks(); void owner.check(); };
  // Returning from the authorization browser resumes bounded observation.
  // These reads never resubmit a code, restart login or probe the Provider.
  useEffect(() => {
    if (requestMismatch || !view.pending || view.busy || loginFailure(view) !== undefined ||
      (!view.error && !["active", "binding_pending"].includes(view.observation?.status ?? ""))) return;
    const resume = () => {
      if (document.visibilityState !== "visible") return;
      pollingUntil.current = Date.now() + 5 * 60_000;
      setChecksPaused(false);
      void owner.check();
    };
    document.addEventListener("visibilitychange", resume);
    window.addEventListener("focus", resume);
    const timer = view.error ? undefined : setTimeout(() => {
      if (Date.now() >= pollingUntil.current) { setChecksPaused(true); return; }
      if (document.visibilityState === "visible") void owner.check();
    }, Math.min(2000, Math.max(0, pollingUntil.current - Date.now())));
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", resume);
      window.removeEventListener("focus", resume);
    };
  }, [owner, view, requestMismatch]);
  const challenge = view.events.findLast((event) => event.event === "login_challenge");
  const [clock, setClock] = useState(Date.now);
  useEffect(() => {
    if (!challenge || challenge.expires_at_ms <= Date.now()) return;
    const timer = setTimeout(() => setClock(Date.now()), Math.min(challenge.expires_at_ms - Date.now(), 2_147_483_647));
    return () => clearTimeout(timer);
  }, [challenge]);
  if (requestMismatch) return <Stack spacing={1.5}>
    <Alert severity="warning">Another sign-in is saved in this tab. Continue that attempt, or close this link. No new login has been started.</Alert>
    <Button variant="contained" disabled={view.busy} onClick={() => { setNewSignIn(true); checkStatus(); }}>Continue saved sign-in</Button>
    <Button onClick={onClose}>Back to Cowboy</Button>
  </Stack>;
  if (reference && view.pending && !knownRequest) return <Stack spacing={1.5}>
    {view.error ? <Alert severity="warning">{view.error}</Alert> : <Typography role="status">Checking the original sign-in…</Typography>}
    <Button disabled={view.busy} onClick={() => void owner.resume(reference)}>Check status</Button>
    <Button onClick={onClose}>Back to Cowboy</Button>
  </Stack>;
  const failure = loginFailure(view);
  const link = !failure && !view.pending?.codeState && view.observation?.status === "active" && !view.observation.cancel_requested && !view.pending?.cancelling &&
    challenge && challenge.expires_at_ms > Math.max(clock, Date.now()) ? loginLink(challenge.verification_url) : undefined;
  const done = loginCompleted(view);
  const cancelling = view.pending?.cancelling || view.observation?.cancel_requested;
  const cancellationAcknowledged = view.observation?.cancel_requested === true;
  const codeState = view.pending?.codeState;
  const verifying = !cancelling && !failure && !view.error &&
    (view.observation?.status === "binding_pending" || (view.observation?.status === "active" && Boolean(codeState)));
  const machineName = machines.find((entry) => entry.id === view.pending?.machine)?.display_name || view.pending?.machine;
  const stateLabel: Record<string, string> = {
    active: "Waiting for authorization", completed: "Signed in on this Machine",
    binding_pending: "Verifying account identity", binding_unavailable: "Account binding is unavailable",
    unknown: "Status is unknown. Check the original sign-in before starting another.", inactive: "This sign-in expired or disconnected. Start again for a new authorization link.",
    cancelling: "Cancellation pending", cancellation_requested: "Cancellation requested. The Machine has not confirmed cleanup yet.",
    cancelled: "Cancelled; cleanup is not confirmed", not_started: "Sign-in did not start",
  };
  return <Stack spacing={1.5}>
    {view.pending
      ? <Typography variant="body2" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>{label} · {machineName}</Typography>
      : <Typography variant="body2">Choose where to sign in. Each Machine has its own login.</Typography>}
    {!reference && !view.pending && <TextField select size="small" fullWidth label="Machine" value={machine}
      disabled={view.busy} onChange={(event) => setMachine(event.target.value)}>
      <MenuItem value="" disabled>Select a Machine</MenuItem>
      {machine && !machines.some((entry) => entry.id === machine) &&
        <MenuItem value={machine}>{machine}</MenuItem>}
      {machines.map((entry) => <MenuItem key={entry.id} value={entry.id} disabled={!entry.connected}>{entry.display_name || entry.id}{entry.connected ? "" : " · offline"}</MenuItem>)}
    </TextField>}
    {(error || view.error) && <Alert severity="warning">{error || view.error}</Alert>}
    {view.submissionError && !done && !cancelling && !failure && <Alert severity="warning">{view.submissionError}</Alert>}
    {failure && <Alert severity="error">{failure}</Alert>}
    {options?.native_profile && !view.pending && <Alert severity="success">Already signed in on this Machine. Sign in again only to renew this account’s login.</Alert>}
    {options && !options.available && !view.pending && <Alert severity="info">Sign-in unavailable: {options.reason ?? "native_identity_verification_unavailable"}</Alert>}
    {!reference && !view.pending && options && options.methods.length > 1 && <TextField select label="Sign-in method" size="small" value={method} onChange={(event) => setMethod(event.target.value)}>
      {options.methods.map((entry) => <MenuItem key={entry.id} value={entry.id} disabled={!entry.available} sx={{ whiteSpace: "normal" }}>{entry.label ?? entry.id}{!entry.available ? ` · ${entry.reason ?? "Unavailable"}` : ""}</MenuItem>)}
    </TextField>}
    {!reference && !view.pending && options?.methods.length === 1 && <Typography variant="body2">{options.methods[0]!.label ?? options.methods[0]!.id}</Typography>}
    {done && <Alert severity="success" role="status">Signed in on this Machine</Alert>}
    {view.pending && !failure && !done && <Stack direction="row" alignItems="center" spacing={1} role="status">
      {verifying && !checksPaused && <CircularProgress size={16} aria-label="Checking sign-in" />}
      <Typography variant="body2">
      {cancelling
        ? stateLabel[cancellationAcknowledged ? "cancellation_requested" : "cancelling"]
        : verifying ? codeState === "submitting" ? "Submitting authorization code…"
        : codeState === "unconfirmed" ? "Code delivery is unconfirmed. Checking this sign-in…"
        : "Verifying sign-in…"
        : !view.observation && view.checking ? "Checking sign-in…"
        : view.observation?.status === "active" && !challenge ? "Preparing authorization link…"
        : stateLabel[view.observation?.status ?? "unknown"] ?? "Sign-in needs reconciliation"}
      </Typography>
    </Stack>}
    {checksPaused && !done && !cancelling && <Typography variant="caption" color="text.secondary">Automatic checks paused. Choose Check status to continue.</Typography>}
    {codeState === "unconfirmed" && !done && !cancelling && !failure && <Typography variant="caption" color="text.secondary">
      Check status before trying again. If verification does not finish, choose Cancel sign-in, then Start again for a new authorization link.
    </Typography>}
    {!failure && !cancelling && !codeState && challenge && challenge.expires_at_ms <= Math.max(clock, Date.now()) && view.observation?.status === "active" &&
      <Alert severity="info">The authorization link has expired. Choose Start again to request a new one.</Alert>}
    {link && challenge && <SubscriptionAuthorization
      key={JSON.stringify([challenge.request_id, link, challenge.expires_at_ms])}
      label={label} link={link} challenge={challenge} busy={view.busy} checking={view.checking === true} submit={(code) => owner.submit(code)} />}
    <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap>
      {!reference && !view.pending && <Button variant="contained" disabled={view.busy || !options?.available || !method}
        onClick={() => { if (options) void (async () => {
          resetChecks();
          await owner.start(options, method);
        })(); }}>{options?.native_profile ? "Sign in again" : "Start sign-in"}</Button>}
      {canReplaceLogin(view) && <Button variant="contained" disabled={view.busy} onClick={() => void (async () => {
        const previousMachine = owner.snapshot.pending?.machine;
        await owner.prepareNew();
        if (!owner.snapshot.pending) {
          setOptions(undefined); setMethod(""); setMachine(previousMachine ?? ""); setNewSignIn(true);
          setReadinessRevision((value) => value + 1);
          resetChecks();
        }
      })()}>Start again</Button>}
      {view.pending && !done && <Button disabled={view.busy} onClick={checkStatus}>Check status</Button>}
      {view.pending && !view.pending.request && !view.pending.cancelling && !codeState && view.error && <Button disabled={view.busy} onClick={() => void owner.retry()}>Retry same sign-in</Button>}
      {reference && !view.pending && <Button disabled={view.busy} onClick={() => void owner.resume(reference)}>Check status</Button>}
      {view.pending && !done && !cancellationAcknowledged && <Button disabled={view.busy} onClick={() => void owner.cancel()}>{view.pending.cancelling ? "Retry cancellation" : "Cancel sign-in"}</Button>}
      {done && <Button variant="contained" disabled={view.busy} onClick={() => { owner.finish(); if (!owner.snapshot.pending) onClose(); }}>Done</Button>}
      {done && <Button disabled={view.busy} onClick={() => {
        owner.finish();
        if (!owner.snapshot.pending) {
          setOptions(undefined); setMethod(""); setMachine(""); setNewSignIn(true); resetChecks();
        }
      }}>Sign in on another Machine</Button>}
    </Stack>
  </Stack>;
}
