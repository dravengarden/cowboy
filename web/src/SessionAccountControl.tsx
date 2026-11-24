import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import WarningAmberRounded from "@mui/icons-material/WarningAmberRounded";
import { useEffect, useState } from "react";
import {
  type AcknowledgementTarget, acknowledgementTarget, acknowledgeUncertainEffect, type AccountControl, accountControl,
  accountStateBadge, type ParkedReason, sessionParkedReason, signInObservation, type UncertainEffect, uncertainEffectGuidance,
} from "./accountControls";
import { glyphSize, desktopSize } from "./surface/desktopSize";
import { mobileComposerPanelHeaderMinHeight } from "./mobileComposerPrimitives";
import type { SessionMeta } from "./protocol";
import { ConfirmSheet } from "./Sheet";
import { useStoreSelector } from "./store";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { sessionAccountIdentity } from "./accountIdentity";
import { useNativeAccountUsage } from "./nativeAccountUsage";
import { providerName } from "./providerPresentation";

/** The governed account of a session and why its queue is parked. */
export function useSessionAccountControl(session: SessionMeta | undefined): {
  control: AccountControl | undefined; parked: ParkedReason | undefined; title: string; refresh: () => Promise<void>;
} {
  const { data, refresh } = useSubscriptionAccounts();
  const scoped = session && data?.dataset.user_id === session.owner_user_id ? data : undefined;
  const control = session ? accountControl(scoped, session.provider_account_id) : undefined;
  const wait = session?.resource_wait;
  const model = useStoreSelector((state) => {
    const value = session ? state.configOptions.get(session.id)?.find((option) => option.id === "model")?.currentValue : undefined;
    return typeof value === "string" ? value : undefined;
  });
  // The queue's wait is pushed with the session; the controls are not.
  useEffect(() => { if (control && wait !== undefined) void refresh(); }, [wait, control !== undefined, refresh]);
  // Only a queued prompt is parked; an uncertain effect parks the session
  // even before the owner sends again.
  const reason = control && session ? sessionParkedReason(control, session.id, wait, Date.now(), model) : undefined;
  const parked = reason && (wait || reason.uncertain) ? reason : undefined;
  const identity = session ? sessionAccountIdentity(data, session) : undefined;
  const provider = session ? providerName(session.provider, session.provider_version, session.provider_generation_digest) : "";
  return { control, parked, title: `${provider} · ${identity?.label ?? ""}`, refresh };
}

/** Acknowledge one uncertain target effect after the owner checked it. The
 * sheet acknowledges exactly the session, method and sequence it showed when
 * it opened; a change meanwhile invalidates it. */
export function UncertainEffectAcknowledge({ session, effect, onDone, size = "small" }: {
  session: string; effect: UncertainEffect; onDone: () => void; size?: "small" | "medium";
}): React.JSX.Element {
  const [shown, setShown] = useState<AcknowledgementTarget | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const target = acknowledgementTarget(shown, session, effect);
  const stale = shown !== undefined && target === undefined;
  function open() { setError(""); setShown({ session, method: effect.method, sequence: effect.sequence }); }
  async function acknowledge(target: AcknowledgementTarget) {
    setBusy(true); setError("");
    try { await acknowledgeUncertainEffect(target.session, target.sequence); setShown(undefined); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Not acknowledged."); }
    finally { setBusy(false); onDone(); }
  }
  return <>
    <Button size={size} onClick={open} data-uncertain-effect-acknowledge={effect.sequence}>Acknowledge…</Button>
    <ConfirmSheet open={shown !== undefined} onClose={() => setShown(undefined)} title="Acknowledge an uncertain effect?"
      actions={<Stack direction="row" spacing={1}>
        <Button onClick={() => setShown(undefined)}>Cancel</Button>
        <Button variant="contained" disabled={busy || !target} onClick={() => target && void acknowledge(target)}>I checked the target</Button>
      </Stack>}>
      <Stack spacing={1}>
        <Typography variant="body2">
          The reply to a <Box component="code">{shown?.method ?? effect.method}</Box> request was lost, so Cowboy cannot tell whether it changed its target.
        </Typography>
        <Typography variant="body2">
          {uncertainEffectGuidance(shown?.method ?? effect.method).check} Acknowledging releases this session's queue; Cowboy never resends the request.
        </Typography>
        {stale && <Alert severity="warning" data-uncertain-effect-changed>The uncertain effect changed. Close this and review it again.</Alert>}
        {error && <Alert severity="warning">{error}</Alert>}
      </Stack>
    </ConfirmSheet>
  </>;
}

/** Session Settings: the account's state and why its queue is parked. */
export function SessionAccountState({ session }: { session: SessionMeta }): React.JSX.Element | null {
  const { control, parked, refresh } = useSessionAccountControl(session);
  const { accounts } = useNativeAccountUsage();
  if (!control) return null;
  const badge = accountStateBadge(control, signInObservation(accounts.find((item) => item.account_id === control.account_id)));
  return <Stack spacing={0.5} data-session-account-state={control.state}>
    <Typography variant="caption" color={badge.tone === "success" ? "success.main" : badge.tone === "default" ? "text.secondary" : "warning.main"}>
      {badge.label}{badge.detail ? ` · ${badge.detail}` : ""}{control.admitted ? "" : " · Not admitted"}
    </Typography>
    {parked && <Typography variant="caption" color="warning.main" data-session-parked>{parked.text}</Typography>}
    {parked?.uncertain && <Box><UncertainEffectAcknowledge session={session.id} effect={parked.uncertain} onDone={() => void refresh()} /></Box>}
  </Stack>;
}

/** The composer's register line for a parked governed session; paint-only
 * like the quota line it sits beside. */
export function AccountParkedStatus({ sessionId, desktop }: { sessionId: string; desktop: boolean }): React.JSX.Element | null {
  const session = useStoreSelector((state) => state.sessions.find((item) => item.id === sessionId));
  const { parked, title, refresh } = useSessionAccountControl(session);
  if (!session || !parked) return null;
  return <Stack data-account-parked-status data-composer-stack-slot="quota" role="status" aria-live="polite"
    direction="row" alignItems="center" spacing={0.75}
    sx={{
      flexShrink: 0, width: "100%", minWidth: 0, boxSizing: "border-box", px: desktop ? 1 : 1.5,
      minHeight: desktop ? desktopSize(28) : mobileComposerPanelHeaderMinHeight,
      ...(desktop && { borderBottom: 1, borderColor: "divider" }),
    }}>
    <WarningAmberRounded sx={{ fontSize: glyphSize(16), flexShrink: 0, color: "warning.main" }} />
    <Typography variant="caption" component="span" sx={{ flex: 1, minWidth: 0, color: "text.secondary", overflowWrap: "anywhere" }}>
      <Box component="span" sx={{ color: "text.primary", fontWeight: 650 }}>{title}</Box>
      {" · "}{parked.text}
    </Typography>
    {parked.uncertain && <UncertainEffectAcknowledge session={session.id} effect={parked.uncertain} onDone={() => void refresh()} />}
  </Stack>;
}
