import { Alert, Button, Stack, TextField, Typography } from "@mui/material";
import { useState } from "react";
import { copyText } from "./clipboard";
import type { ProviderLoginEvent } from "./providerAuthenticationOwner";
import { LinkBrowserEntry } from "./machineBrowser/LinkBrowserEntry";
import type { LoginScope } from "./subscriptionLoginController";

/** Mounted for one account/request/challenge only. Authorization material stays
 * in this component, never in account metadata or browser recovery storage.
 *
 * The dialog never opens the link in a browser of this device (owner
 * decision 2026-11-11): the person copies it and pastes it into the
 * account's Machine browser, so the sign-in happens where the credentials
 * are used. */
export function SubscriptionAuthorization({ label, link, challenge, busy, checking, submit, scope, machine }: {
  label: string; link: string;
  challenge: Extract<ProviderLoginEvent, { event: "login_challenge" }>;
  busy: boolean; checking: boolean; submit: (code: string) => Promise<void>;
  /** With the login's Machine, offers to show `link` in the account's
   * ordinary Machine browser there. The code field stays the person's. */
  scope?: LoginScope | undefined;
  machine?: string | undefined;
}): React.JSX.Element {
  const [code, setCode] = useState("");
  const [notice, setNotice] = useState("");
  const [manualCopy, setManualCopy] = useState(false);
  const expired = () => challenge.expires_at_ms <= Date.now();
  return <Stack spacing={1.5} data-subscription-authorization={label}>
    <Typography variant="body2">
      Copy the sign-in link and paste it into the Machine browser of <strong>{label}</strong>.
      On Claude’s page, check that you are authorizing this account.
    </Typography>
    <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
      <Button variant="contained" disabled={busy} onClick={() => {
        if (expired()) return;
        setNotice("");
        void copyText(link).then((copied) => {
          setManualCopy(!copied);
          if (copied) setNotice("Link copied.");
        });
      }}>Copy sign-in link</Button>
      {scope && machine && <LinkBrowserEntry owner={scope.owner} dataset={scope.dataset} account={scope.account}
        label={label} machine={machine} url={link} disabled={busy} />}
    </Stack>
    {notice && <Typography role="status" variant="caption">{notice}</Typography>}
    {manualCopy && <Stack spacing={1}>
      <Alert severity="warning">Could not copy automatically. Select the link below and copy it.</Alert>
      <TextField fullWidth size="small" label="Sign-in link" value={link}
        slotProps={{ htmlInput: { readOnly: true, autoComplete: "off", spellCheck: false, style: { fontSize: 16 } } }}
        onFocus={(event) => event.target.select()} />
    </Stack>}
    <Typography variant="caption" color="text.secondary">
      In the account’s Browser…, choose More → Paste from this device…, then Open as address.
      {" "}Do not open the link in a regular tab on this device: it shares that browser’s Claude login.
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
