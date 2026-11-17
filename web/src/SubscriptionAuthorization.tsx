import { Alert, Button, Stack, TextField, Typography } from "@mui/material";
import { useState } from "react";
import type { ProviderLoginEvent } from "./providerAuthenticationOwner";
import { LinkBrowserEntry } from "./machineBrowser/LinkBrowserEntry";
import type { LoginScope } from "./subscriptionLoginController";

/** Mounted for one account/request/challenge only. Authorization material stays
 * in this component, never in account metadata or browser recovery storage.
 *
 * The dialog never opens, shows or copies the link on this device (owner
 * decisions 2026-11-11 and 2026-11-17): it only offers the account's Machine
 * browser on the login's Machine, so every request of the Provider page and
 * every fingerprint it can read come from that Machine. Without a scope and
 * Machine there is no way to show the link, and the dialog says so. */
export function SubscriptionAuthorization({ label, link, challenge, busy, checking, submit, scope, machine }: {
  label: string; link: string;
  challenge: Extract<ProviderLoginEvent, { event: "login_challenge" }>;
  busy: boolean; checking: boolean; submit: (code: string) => Promise<void>;
  /** With the login's Machine, shows `link` in the account's ordinary
   * Machine browser there. The code field stays the person's. */
  scope?: LoginScope | undefined;
  machine?: string | undefined;
}): React.JSX.Element {
  const [code, setCode] = useState("");
  const expired = () => challenge.expires_at_ms <= Date.now();
  return <Stack spacing={1.5} data-subscription-authorization={label}>
    <Typography variant="body2">
      Open the sign-in page in the Machine browser of <strong>{label}</strong>.
      On Claude’s page, check that you are authorizing this account.
    </Typography>
    {scope && machine
      ? <LinkBrowserEntry owner={scope.owner} dataset={scope.dataset} account={scope.account}
        label={label} machine={machine} url={link} disabled={busy} />
      : <Alert severity="warning">This sign-in has no Machine browser to open it in. Cancel it and start again.</Alert>}
    <Typography variant="caption" color="text.secondary">
      The page runs on the Machine, not on this device. Copy the code it shows with the browser’s clipboard, then return here.
    </Typography>
    {challenge.user_code && <Typography component="code">{challenge.user_code}</Typography>}
    {challenge.input_required && <Stack spacing={1}>
      <TextField fullWidth size="small" label={`Authorization code for ${label}`} value={code} autoComplete="off" type="password"
        helperText="Return here with the code from this sign-in. Do not use a code from another account or an older attempt."
        disabled={busy && !checking} onChange={(event) => setCode(event.target.value)} />
      <Button variant="contained" sx={{ alignSelf: "flex-start" }} disabled={busy || !code.trim()}
        onClick={() => {
          if (expired()) return;
          const input = code; setCode(""); void submit(input);
        }}>Continue</Button>
    </Stack>}
  </Stack>;
}
