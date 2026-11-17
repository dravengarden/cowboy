import { Alert, Button, Stack, TextField } from "@mui/material";
import { useState } from "react";
import { ProviderAccountStatus } from "./ProviderAccountIdentity";
import { productSessionSignal } from "./productSessionEnd";
import type { PortableCredential } from "./subscriptionAccounts";

/** The owner-facing state of a portable account's Service-held API key. */
export function apiKeyStatus(credential: PortableCredential | undefined): { label: string; tone: "success" | "warning"; detail?: string } {
  if (!credential) return { label: "No API key", tone: "warning", detail: "Enter one to use this account" };
  if (credential.authentication_state !== "ready") return { label: "API key needs attention", tone: "warning", detail: "Enter a new key" };
  return {
    label: "API key saved",
    tone: "success",
    detail: credential.distribution_state === "current" ? "Delivered to Machines" : "Delivery to Machines pending",
  };
}

const ERRORS: Record<string, string> = {
  invalid_secret: "Enter a single API key without line breaks.",
  account_unavailable: "This account is disabled or being removed.",
  authentication_method_unavailable: "This Provider no longer accepts this key type. Refresh and try again.",
  portable_account_provider_unavailable: "This Provider no longer accepts account API keys.",
  credential_rejected: "The key does not match this Provider's credential contract.",
};
/** Never includes the key: the response carries only a code. */
export function apiKeyErrorText(code: string | undefined): string {
  return (code && ERRORS[code]) || "The key was not saved. Try again.";
}

/** A portable account's API key: sent once to the Service, which keeps it in
 * this account's own vault and delivers it to Machines. It is never shown again. */
export function SubscriptionApiKey({ account, method, credential, disabled, expanded, onChange }: {
  account: string; method: { id: string; label: string } | undefined; credential: PortableCredential | undefined;
  disabled: boolean; expanded: boolean; onChange: () => void;
}): React.JSX.Element {
  const [secret, setSecret] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const status = apiKeyStatus(credential);
  async function save() {
    if (!method || !secret.trim()) return;
    setBusy(true); setError("");
    try {
      const response = await fetch(`/api/provider-accounts/${encodeURIComponent(account)}/credentials`, {
        method: "POST", headers: { "Content-Type": "application/json" }, cache: "no-store",
        signal: productSessionSignal(), body: JSON.stringify({ method: method.id, secret }),
      });
      if (response.ok) { setSecret(""); onChange(); return; }
      const body = await response.json().catch(() => undefined) as { error?: { code?: string } } | undefined;
      setError(apiKeyErrorText(body?.error?.code));
    } catch { setError(apiKeyErrorText(undefined)); }
    finally { setBusy(false); }
  }
  return <Stack spacing={0.5}>
    <ProviderAccountStatus label={status.label} tone={status.tone} {...(status.detail ? { detail: status.detail } : {})} />
    {error && <Alert severity="warning">{error}</Alert>}
    {expanded && <Stack direction="row" spacing={1} component="form" onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <TextField size="small" type="password" label={method?.label ?? "API key"} value={secret} disabled={disabled || busy || !method}
        autoComplete="off" fullWidth sx={{ minWidth: 0 }} slotProps={{ htmlInput: { spellCheck: false, "data-subscription-api-key": account } }}
        onChange={(event) => setSecret(event.target.value)} />
      <Button type="submit" disabled={disabled || busy || !method || !secret.trim()}>{credential ? "Replace key" : "Save key"}</Button>
    </Stack>}
  </Stack>;
}
