import { Alert, Button, Stack, TextField, Typography } from "@mui/material";
import { useRef, useState } from "react";
import { copyText } from "./clipboard";
import { hasNativeAuthenticationBrowser, hasNativeExternalOpener, openAccountAuthorizationUrl } from "./openExternal";
import type { ProviderLoginEvent } from "./providerAuthenticationOwner";

/** Mounted for one account/request/challenge only. Authorization material stays
 * in this component, never in account metadata or browser recovery storage. */
export function SubscriptionAuthorization({ label, link, challenge, busy, checking, submit }: {
  label: string; link: string;
  challenge: Extract<ProviderLoginEvent, { event: "login_challenge" }>;
  busy: boolean; checking: boolean; submit: (code: string) => Promise<void>;
}): React.JSX.Element {
  const [code, setCode] = useState("");
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const [opening, setOpening] = useState(false);
  const openingRef = useRef(false);
  const systemBrowser = hasNativeExternalOpener();
  const unsupportedShell = hasNativeAuthenticationBrowser() && !systemBrowser;
  const expired = () => challenge.expires_at_ms <= Date.now();
  return <Stack spacing={1.5} data-subscription-authorization={label}>
    <Typography variant="body2">
      On Claude’s page, check that you are authorizing the account for <strong>{label}</strong>.
    </Typography>
    <Stack direction="row" spacing={1} useFlexGap flexWrap="wrap">
      <Button component="a" href={link} target="_blank" rel="noopener noreferrer"
        variant="contained" disabled={busy || opening || unsupportedShell}
        onClick={(event) => {
          if (busy || openingRef.current || expired()) { event.preventDefault(); return; }
          if (!systemBrowser) return; // Keep Safari/PWA's native user-gesture navigation.
          event.preventDefault();
          openingRef.current = true; setOpening(true); setError("");
          void openAccountAuthorizationUrl(link).catch((cause: unknown) => {
            setError(cause instanceof Error ? cause.message : "Could not open the browser. Copy the link instead.");
          }).finally(() => { openingRef.current = false; setOpening(false); });
        }}>{opening ? "Opening browser…" : "Open Claude sign-in"}</Button>
      <Button disabled={busy} onClick={() => {
        if (expired()) return;
        setNotice(""); setError("");
        void copyText(link).then((copied) => {
          if (copied) setNotice("Link copied. Open it in the browser profile for this account, then return here.");
          else setError("Could not copy the link. Press and hold Open Claude sign-in to copy its link.");
        });
      }}>Copy sign-in link</Button>
    </Stack>
    <Typography variant="caption" color="text.secondary">
      Another Claude account already open? Use a separate browser profile, or close other private sign-in tabs and paste the link into a new private session.
      {" "}A new regular tab shares the browser’s Claude login.
    </Typography>
    {unsupportedShell && <Alert severity="info">Copy the link to your browser to keep this account out of the shared sign-in window.</Alert>}
    {notice && <Typography role="status" variant="caption">{notice}</Typography>}
    {error && <Alert severity="warning">{error}</Alert>}
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
