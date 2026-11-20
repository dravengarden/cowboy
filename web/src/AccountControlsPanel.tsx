import { Alert, Button, FormControlLabel, Stack, Switch, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import {
  type AccountControl, accountPlanLabel, accountStateBadge, accountUsageSummary, type ControlAction, controlActions, controlReasonText, logoutStatus,
  postAccountControl, sessionPlanKey, signInObservation,
} from "./accountControls";
import { ProviderAccountStatus } from "./ProviderAccountIdentity";
import { ConfirmSheet } from "./Sheet";
import { useStoreSelector } from "./store";

const CONFIRM: Partial<Record<ControlAction, { title: string; body: string; button: string }>> = {
  block: {
    title: "Block this account?",
    body: "Blocking parks every queue of this account at once and stops all of its units on every Machine. A running turn is interrupted. Nothing is sent again until you unblock it.",
    button: "Block",
  },
  logout: {
    title: "Sign out this account?",
    body: "Runs the Provider's own sign-out in this account's sandbox on each signed-in Machine and removes only its credential. Its conversations and history stay, and the account remains. An offline Machine signs out when it reconnects. Signing in again needs the account unblocked.",
    button: "Sign out",
  },
};

/** Debounce for reloading account controls after a session reports a plan. */
const PLAN_REFRESH_MS = 1_000;

/** Account controls of a named-only (Antigravity) account card: state,
 * the session using it, usage, admission and the owner's kill switch. */
export function AccountControlsPanel({ control, observation, disabled, onChange }: {
  control: AccountControl;
  /** The account's cached usage/sign-in observation, if any. */
  observation: Parameters<typeof signInObservation>[0] & Parameters<typeof accountUsageSummary>[0];
  disabled: boolean;
  onChange: () => void;
}): React.JSX.Element {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirming, setConfirming] = useState<ControlAction>();
  const badge = accountStateBadge(control, signInObservation(observation));
  const actions = controlActions(control);
  const pendingFences = control.fences.filter((fence) => fence.state !== "confirmed");
  const logout = logoutStatus(control.logout);
  const plan = accountPlanLabel(control);
  // A session broadcast reporting another plan for this account refreshes
  // the account's controls (debounced), so the card never keeps a stale plan.
  const reported = useStoreSelector((state) => sessionPlanKey(state.sessions, control.account_id));
  const seen = useRef(reported);
  const refresh = useRef(onChange);
  refresh.current = onChange;
  useEffect(() => {
    if (seen.current === reported) return;
    seen.current = reported;
    const timer = setTimeout(() => refresh.current(), PLAN_REFRESH_MS);
    return () => clearTimeout(timer);
  }, [reported]);
  async function run(action: ControlAction) {
    setBusy(true); setError("");
    try {
      await postAccountControl(control.account_id, action, control);
      setConfirming(undefined);
    } catch (cause) { setError(cause instanceof Error ? cause.message : "Action refused."); }
    finally { setBusy(false); onChange(); }
  }
  const off = disabled || busy;
  const confirm = confirming ? CONFIRM[confirming] : undefined;
  return <Stack spacing={0.75} data-account-controls={control.account_id} data-account-state={control.state}>
    <ProviderAccountStatus label={badge.label} tone={badge.tone === "success" ? "success" : badge.tone === "default" ? "default" : "warning"}
      {...(badge.detail ? { detail: badge.detail } : {})} />
    {plan && <Typography variant="caption" color="text.secondary" data-account-plan={control.plan?.id}
      title="Reported by the Provider during a session of this account">Plan · {plan}</Typography>}
    <Typography variant="caption" color="text.secondary" data-account-usage>{accountUsageSummary(observation)}</Typography>
    {pendingFences.length > 0 && <Typography variant="caption" color="warning.main">
      Stop pending on {pendingFences.map((fence) => fence.machine_id).join(", ")}
    </Typography>}
    {logout && <Typography variant="caption" color={control.logout?.state === "signed_out" ? "text.secondary" : "warning.main"} data-account-logout={control.logout?.state}>{logout}</Typography>}
    <Typography variant="caption" color="text.secondary" data-account-notice>
      Use of a personal Google account in Cowboy is not supported by Google.
    </Typography>
    <FormControlLabel sx={{ ml: 0 }} disabled={off}
      control={<Switch size="small" checked={control.admitted} onChange={(event) => void run(event.target.checked ? "admit" : "unadmit")} />}
      label={<Typography variant="caption">{control.admitted ? "Admitted" : "Not admitted"}</Typography>} />
    <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
      {actions.clear && <Button size="small" disabled={off} onClick={() => void run("clear")}
        title={controlReasonText(control.reason)}>Clear state</Button>}
      {actions.block && <Button size="small" color="error" disabled={off} onClick={() => setConfirming("block")}>Block…</Button>}
      {actions.unblock && <Button size="small" disabled={off} onClick={() => void run("unblock")}>Unblock</Button>}
      {actions.logout && <Button size="small" disabled={off} onClick={() => setConfirming("logout")}>Sign out…</Button>}
      {actions.stopUnits && <Button size="small" disabled={off} onClick={() => void run("stop_units")}>Stop units</Button>}
    </Stack>
    {actions.clear && controlReasonText(control.reason) && <Typography variant="caption" color="text.secondary">
      Reason · {controlReasonText(control.reason)}
    </Typography>}
    {error && <Alert severity="warning">{error}</Alert>}
    <ConfirmSheet open={Boolean(confirm)} onClose={() => setConfirming(undefined)} title={confirm?.title}
      actions={<Stack direction="row" spacing={1}>
        <Button onClick={() => setConfirming(undefined)}>Cancel</Button>
        <Button color="error" variant="contained" disabled={busy} onClick={() => confirming && void run(confirming)}>{confirm?.button}</Button>
      </Stack>}>
      <Typography variant="body2">{confirm?.body}</Typography>
    </ConfirmSheet>
  </Stack>;
}
