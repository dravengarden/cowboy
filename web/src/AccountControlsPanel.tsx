import { Alert, Button, FormControlLabel, Link, Stack, Switch, Typography } from "@mui/material";
import { useState } from "react";
import {
  type AccountControl, accountStateBadge, type ControlAction, controlActions, controlReasonText, logoutStatus,
  postAccountControl, signInObservation,
} from "./accountControls";
import { ProviderAccountStatus } from "./ProviderAccountIdentity";
import { ConfirmSheet } from "./Sheet";
import { requestPickSession } from "./sessionPickRequest";
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
  force_release: {
    title: "Force release the active turn?",
    body: "The account's units are confirmed stopped, but the turn's outcome could not be settled. Releasing lets another turn of this account start; the released turn is not resent.",
    button: "Force release",
  },
};

/** Account controls of a named-only (Antigravity) account card: state,
 * active turn, usage, admission and the owner's kill switch. */
export function AccountControlsPanel({ control, observation, disabled, onChange }: {
  control: AccountControl;
  /** The account's cached usage/sign-in observation, if any. */
  observation: { reason?: string | undefined; available?: boolean | undefined } | undefined;
  disabled: boolean;
  onChange: () => void;
}): React.JSX.Element {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirming, setConfirming] = useState<ControlAction>();
  const sessions = useStoreSelector((state) => state.sessions);
  const badge = accountStateBadge(control, signInObservation(observation));
  const actions = controlActions(control);
  const turnSession = control.turn ? sessions.find((session) => session.id === control.turn?.session_id) : undefined;
  const pendingFences = control.fences.filter((fence) => fence.state !== "confirmed");
  const logout = logoutStatus(control.logout);
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
    {control.turn && <Typography variant="caption" color="text.secondary" data-account-turn>
      In use by 1 turn ·{" "}
      <Link component="button" type="button" variant="caption" sx={{ verticalAlign: "baseline" }}
        onClick={() => requestPickSession(control.turn!.session_id, turnSession)}>
        {turnSession?.title || control.turn.session_id}
      </Link>
    </Typography>}
    <Typography variant="caption" color="text.secondary">Usage · Not available</Typography>
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
      {actions.forceRelease && <Button size="small" color="error" disabled={off} onClick={() => setConfirming("force_release")}>Force release…</Button>}
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
