import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { ConfirmSheet } from "./Sheet";
import { SubscriptionLoginForm } from "./SubscriptionLogin";
import { validSubscriptionSnapshot } from "./SubscriptionManagement";
import { productSyncPrincipal } from "./productSyncIdentity";
import { productSessionSignal } from "./productSessionEnd";
import type { LoginScope } from "./subscriptionLoginController";
import { closeSubscriptionSignIn, subscriptionSignInLink } from "./subscriptionSignInLink";
import { signalBootReady } from "./bootSnapshot";

/** Lazy mounted behind ProductAuthGate on both surfaces. Fetches authoritative
 * labels/scope before presenting a reference received from an agent or CLI. */
export function SubscriptionSignInHandoff({ hash }: { hash: string }): React.JSX.Element {
  const [account, setAccount] = useState<{ scope: LoginScope; label: string }>();
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);
  const reference = subscriptionSignInLink(hash);
  const accountId = reference?.account;
  useEffect(() => signalBootReady(), []);
  useEffect(() => {
    const owner = productSyncPrincipal();
    const abort = new AbortController();
    setAccount(undefined); setError("");
    void (async () => {
      try {
        if (!owner || !accountId) throw new Error("Invalid sign-in reference.");
        const response = await fetch("/api/provider-accounts", { cache: "no-store", signal: AbortSignal.any([abort.signal, productSessionSignal()]) });
        if (!response.ok) throw new Error("Could not load this subscription. Check your Cowboy login and retry.");
        const value: unknown = await response.json();
        if (!validSubscriptionSnapshot(value, owner)) throw new Error("Could not verify the subscription owner.");
        const found = value.registry.accounts.find((entry) => entry.id === accountId);
        if (!found) throw new Error("This subscription is not available to the current Cowboy account.");
        if (!abort.signal.aborted && !productSessionSignal().aborted && productSyncPrincipal() === owner) {
          setAccount({ label: found.label, scope: { owner, dataset: value.dataset.dataset_id, account: found.id, provider: found.provider_id } });
        }
      } catch (cause) {
        if (!abort.signal.aborted) setError(cause instanceof Error ? cause.message : "Could not load sign-in.");
      }
    })();
    return () => abort.abort();
  }, [accountId, attempt]);
  return <ConfirmSheet open onClose={closeSubscriptionSignIn} title={account ? `${account.label} · Sign in` : "Subscription sign-in"} wide>
    {account && reference
      ? <SubscriptionLoginForm key={`${hash}:${account.scope.owner}:${account.scope.dataset}`} scope={account.scope} label={account.label} request={reference.request} onClose={closeSubscriptionSignIn}
        onStarted={(request) => {
          history.replaceState(history.state, "", `${location.pathname}${location.search}#subscription-sign-in/${account.scope.account}/${request}`);
          window.dispatchEvent(new Event("hashchange"));
        }} />
      : <Stack spacing={1.5}>{error ? <><Alert severity="warning">{error}</Alert><Button onClick={() => setAttempt((value) => value + 1)}>Retry</Button></> : <Typography role="status">Loading subscription…</Typography>}</Stack>}
  </ConfirmSheet>;
}
