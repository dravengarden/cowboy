import { useState } from "react";
import { Alert, Box, Button, Collapse, Divider, LinearProgress, Stack, Typography } from "@mui/material";
import type { SessionMeta } from "./protocol";
import { sessionAccountIdentity } from "./accountIdentity";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { SubscriptionLogins } from "./SubscriptionLogins";
import { sessionAccountQuota } from "./usageAccounts";
import { sessionProviderUsageRows } from "./sessionSettingsPresentation";
import { useStoreSelector } from "./store";
import { useCallClock } from "./managedCalls";
import { providerName } from "./providerPresentation";

/** Session binding is immutable. In particular a named account must never mount
 * the Service credential editor or consume its Provider-wide quota collector. */
export function SessionAccountDetails({ session }: { session: SessionMeta }): React.JSX.Element {
  const { data, error } = useSubscriptionAccounts();
  const machines = useStoreSelector((state) => state.machines);
  const sessions = useStoreSelector((state) => state.sessions);
  const [manage, setManage] = useState(false);
  const clock = useCallClock();
  const account = sessionAccountIdentity(data, session);
  const scoped = data?.dataset.user_id === session.owner_user_id ? data : undefined;
  const quota = account.named ? sessionAccountQuota(null, scoped, sessions, session) : undefined;
  const rows = quota && quota.observed_at_ms > 0 ? sessionProviderUsageRows(quota, clock) : [];
  const machine = machines.find((item) => item.id === session.machine_id)?.display_name || session.machine_id || "Local";
  return <Stack spacing={0.75} data-session-account={account.id} sx={{ py: 0.75 }}>
    <Divider />
    <Typography variant="body2" sx={{ fontWeight: 650 }}>Account</Typography>
    <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>{account.label}</Typography>
    <Typography variant="caption" color="text.secondary">
      {account.named ? `Bound to this session · AI on ${machine}` : "Service account · Bound to this session"}
    </Typography>
    {error && <Alert severity="warning">{error}</Alert>}
    {account.named && <>
      <Typography variant="caption" color="text.secondary">
        {providerName(session.provider, session.provider_version, session.provider_generation_digest)}
        {session.provider_version ? ` · v${session.provider_version}` : ""}
      </Typography>
      {!account.available && <Typography variant="caption" color="warning.main">{account.description}</Typography>}
      {rows.length ? rows.map((row) => <Box key={row.id}>
        <Stack direction="row" justifyContent="space-between" spacing={1}>
          <Typography variant="caption" color="text.secondary">{row.label}</Typography>
          <Typography variant="caption" sx={{ textAlign: "right" }}>{row.value}</Typography>
        </Stack>
        {row.remaining !== undefined && <LinearProgress variant="determinate" value={row.remaining} aria-label={`${row.label} remaining`} />}
      </Box>) : <Typography variant="caption" color="text.secondary">No quota reported for this account yet.</Typography>}
      {scoped && <Box>
        <Button size="small" aria-expanded={manage} onClick={() => setManage((value) => !value)}>Manage account sign-ins</Button>
        <Collapse in={manage} unmountOnExit>
          <SubscriptionLogins key={account.id} label={account.label} disabled={!account.available} expanded
            scope={{ owner: scoped.dataset.user_id, dataset: scoped.dataset.dataset_id, account: account.id!, provider: session.provider }} />
        </Collapse>
      </Box>}
    </>}
  </Stack>;
}
