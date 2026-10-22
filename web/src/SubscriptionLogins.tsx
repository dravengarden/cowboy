import { Alert, Button, Stack, Typography } from "@mui/material";
import { useCallback, useEffect, useRef, useState } from "react";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import { useStoreSelector } from "./store";
import { SubscriptionLogin } from "./SubscriptionLogin";
import type { LoginScope } from "./subscriptionLoginController";
import { ProviderAccountStatus } from "./ProviderAccountIdentity";

interface Login {
  request_id: string; account_id: string; provider_id: string; machine_id: string; status: string; cancel_requested: boolean;
  /** The Machine now runs another Provider release; this login no longer admits sessions there. */
  outdated?: { signed_in_version: string; installed_version: string };
}
const outdated = (login: Login) => login.status === "completed" && Boolean(login.outdated);
interface Page { logins: Login[]; next_after_machine: string | null }
export function subscriptionLoginsPage(value: unknown, scope: LoginScope, after = ""): Page {
  const body = value as (Page & { account_id: string; provider_id: string }) | null;
  const id = (value: unknown): value is string => typeof value === "string" && /^[A-Za-z0-9_-]{1,512}$/.test(value);
  if (!body || body.account_id !== scope.account || body.provider_id !== scope.provider || !Array.isArray(body.logins) || body.logins.length > 50 ||
    !(body.next_after_machine === null || id(body.next_after_machine))) throw new Error("Invalid login list.");
  let previous = after;
  for (const login of body.logins) {
    if (!login || login.account_id !== scope.account || login.provider_id !== scope.provider || !id(login.request_id) || !id(login.machine_id) ||
      login.machine_id <= previous || typeof login.status !== "string" || typeof login.cancel_requested !== "boolean") throw new Error("Login list does not match this account.");
    previous = login.machine_id;
  }
  if (body.next_after_machine !== null && (body.logins.length === 0 || body.next_after_machine !== previous)) throw new Error("Invalid login cursor.");
  return { logins: body.logins, next_after_machine: body.next_after_machine };
}
const status: Record<string, string> = {
  active: "Waiting for sign-in", completed: "Signed in", inactive: "Sign-in expired or disconnected",
  unknown: "Status needs checking", binding_pending: "Verifying account", binding_unavailable: "Sign-in needs attention",
  cancelling: "Cancellation pending", cancellation_requested: "Cancellation requested", not_started: "Not signed in",
};

/** Local DB metadata only. No polling, native login or Provider probe on mount. */
export function SubscriptionLogins({ scope, label, disabled, expanded }: { scope: LoginScope; label: string; disabled: boolean; expanded: boolean }): React.JSX.Element {
  const machines = useStoreSelector((state) => state.machines);
  const [page, setPage] = useState<Page>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const sequence = useRef(0);
  const load = useCallback(async (after = "") => {
    const current = ++sequence.current;
    setBusy(true); setError("");
    try {
      const response = await fetch(`/api/provider-accounts/${scope.account}/logins?limit=20&after_machine=${encodeURIComponent(after)}`, { cache: "no-store", signal: productSessionSignal() });
      if (!response.ok) throw new Error("Could not load sign-ins.");
      const next = subscriptionLoginsPage(await response.json(), scope, after);
      if (sequence.current === current && productSyncPrincipal() === scope.owner && !productSessionSignal().aborted) {
        setPage((old) => ({ logins: after ? [...(old?.logins ?? []), ...next.logins] : next.logins, next_after_machine: next.next_after_machine }));
      }
    } catch { if (sequence.current === current) setError("Sign-in status is unavailable. Refresh to check again."); }
    finally { if (sequence.current === current) setBusy(false); }
  }, [scope.account, scope.dataset, scope.owner, scope.provider]);
  useEffect(() => { void load(); return () => { ++sequence.current; }; }, [load]);
  return <Stack spacing={0.5}>
    {error && <Alert severity="warning">{error}</Alert>}
    {!page && !error && <Typography variant="body2" color="text.secondary">Loading sign-ins…</Typography>}
    {page?.logins.length === 0 && <ProviderAccountStatus label="Not signed in" tone="warning" detail="No Machine" />}
    {page?.logins.map((login) => <Stack key={login.machine_id} direction="row" spacing={1} alignItems="center" justifyContent="space-between" flexWrap="wrap" useFlexGap>
      <ProviderAccountStatus
        label={outdated(login) ? "Sign in again" : status[login.status] ?? "Sign-in needs attention"}
        tone={login.status === "completed" && !outdated(login) ? "success" : "warning"}
        detail={[
          machines.find((machine) => machine.id === login.machine_id)?.display_name || login.machine_id,
          login.outdated && outdated(login)
            ? `Provider updated ${login.outdated.signed_in_version} → ${login.outdated.installed_version}`
            : undefined,
        ].filter(Boolean).join(" · ")} />
      {expanded && <SubscriptionLogin scope={scope} label={label} request={login.request_id} status={login.status} disabled={busy} onChange={() => void load()} />}
    </Stack>)}
    {expanded && <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
      <SubscriptionLogin scope={scope} label={label} disabled={disabled || busy}
        startLabel={page?.logins.some(outdated) ? "Sign in again…"
          : page?.logins.some((login) => login.status === "completed") ? "Sign in on another Machine…" : "Sign in…"} onChange={() => void load()} />
      <Button disabled={busy} onClick={() => void load()}>Refresh status</Button>
      {page?.next_after_machine && <Button disabled={busy} onClick={() => void load(page.next_after_machine!)}>More Machines</Button>}
    </Stack>}
  </Stack>;
}
