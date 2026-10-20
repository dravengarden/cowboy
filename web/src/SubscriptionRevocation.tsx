import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { ConfirmSheet } from "./Sheet";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";

interface Progress {
  status: "not_started" | "revoking" | "completed";
  account_id: string;
  completion?: { account_completed: boolean; missing: { kind: string; target: string }[] };
}
interface Pending { operation: string; dataset: string; expected_revision: number }
export function validRevocationPending(value: unknown, dataset: string): value is Pending {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const input = value as Record<string, unknown>;
  return Object.keys(input).every((key) => ["operation", "dataset", "expected_revision"].includes(key)) &&
    typeof input.operation === "string" && /^[A-Za-z0-9_-]{1,128}$/.test(input.operation) && input.dataset === dataset &&
    Number.isSafeInteger(input.expected_revision) && Number(input.expected_revision) >= 0;
}
export function revocationProgress(value: unknown, account: string): Progress {
  const input = value as Progress | null;
  if (!input || input.account_id !== account || !["not_started", "revoking", "completed"].includes(input.status) ||
    (input.status !== "not_started" && (!input.completion || typeof input.completion.account_completed !== "boolean" ||
      !Array.isArray(input.completion.missing) || !input.completion.missing.every((item) => item && typeof item.kind === "string" && typeof item.target === "string"))) ||
    (input.status === "completed" && (!input.completion?.account_completed || input.completion.missing.length !== 0))) {
    throw new Error("Invalid revocation status. Completion is not confirmed.");
  }
  return input;
}

export function SubscriptionRevocation({ account, label, revoking, owner, dataset, revision, disabled, onChange }: {
  account: string; label: string; revoking: boolean; owner: string; dataset: string; revision: number;
  disabled: boolean; onChange: () => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const [progress, setProgress] = useState<Progress>();
  const [pending, setPending] = useState<Pending>();
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(false);
  const locked = useRef(false);
  const key = `cowboy:subscription-revocation:${owner}:${account}`;
  const path = `/api/provider-accounts/${account}/revocation`;
  const current = () => alive.current && !productSessionSignal().aborted && productSyncPrincipal() === owner;
  useEffect(() => {
    alive.current = true;
    try {
      const saved = sessionStorage.getItem(key);
      if (saved) {
        const input: unknown = JSON.parse(saved);
        if (!validRevocationPending(input, dataset)) throw new Error();
        setPending(input);
      }
      setReady(true);
    } catch { setError("Saved revocation could not be read. No new operation will start."); }
    return () => { alive.current = false; };
  }, [key, dataset]);
  async function read() {
    const response = await fetch(path, { cache: "no-store", signal: productSessionSignal() });
    const body: unknown = await response.json();
    if (!response.ok) {
      if (current()) setProgress(undefined);
      const code = (body as { error?: { code?: unknown } } | null)?.error?.code;
      throw new Error(code === "account_not_found" ? "This account is no longer available. Reopen subscriptions." :
        code === "revocation_plan_ambiguous" ? "Cleanup has conflicting plans and requires repair." : "Could not read cleanup status.");
    }
    const value = revocationProgress(body, account);
    if (current()) { setProgress(value); if (value.status !== "not_started") onChange(); }
  }
  async function run(action: "read" | "start" | "resume") {
    if (!ready || locked.current || !current()) return;
    locked.current = true; setBusy(true); setError("");
    try {
      if (action !== "read") {
        let input: Pending | { dataset: string };
        if (action === "start") {
          input = pending ?? { operation: crypto.randomUUID(), dataset, expected_revision: revision };
          if (!validRevocationPending(input, dataset)) throw new Error("Account dataset changed.");
          sessionStorage.setItem(key, JSON.stringify(input));
          setPending(input);
        } else { input = { dataset }; }
        const response = await fetch(path + (action === "resume" ? "/reconcile" : ""), {
          method: "POST", headers: { "Content-Type": "application/json" }, signal: productSessionSignal(), body: JSON.stringify(input),
        });
        const value = await response.json() as { error?: { code?: string }; account_id?: string; status?: string };
        if (!current()) return;
        if (!response.ok) {
          if (action === "start" && response.status === 409 && ["revision_conflict", "account_revoking", "account_not_found"].includes(value.error?.code ?? "")) {
            sessionStorage.removeItem(key); setPending(undefined); onChange();
            await read();
            if (value.error?.code === "account_revoking") return;
            throw new Error("Subscriptions changed. Review the refreshed list and confirm again.");
          }
          await read();
          throw new Error("Cleanup is not confirmed. Check status or retry the same operation.");
        }
        if (action === "start") {
          if (value.account_id !== account || !["revoking", "completed"].includes(value.status ?? "")) throw new Error("Revocation response does not match this account.");
          sessionStorage.removeItem(key); setPending(undefined);
        }
      }
      await read();
    } catch (cause) { if (current()) setError(`Cleanup is not confirmed. ${cause instanceof Error ? cause.message : "Check status or retry the same operation."}`); }
    finally { locked.current = false; if (current()) setBusy(false); }
  }
  const started = revoking || (progress && progress.status !== "not_started");
  const completed = progress?.status === "completed";
  return <>
    <Button color={revoking ? "inherit" : "error"} disabled={disabled || (!ready && !error)} onClick={() => { setOpen(true); void run("read"); }}>
      {revoking ? "Cleanup status…" : pending ? "Resume removal…" : "Remove account…"}
    </Button>
    <ConfirmSheet open={open} onClose={() => setOpen(false)} title={`${label} · ${started ? "Cleanup" : "Remove account?"}`}
      actions={<Stack direction="row" spacing={1}>
        <Button onClick={() => setOpen(false)}>Close</Button>
        {started ? <>
          <Button disabled={busy} onClick={() => void run("read")}>Check status</Button>
          {!completed && <Button disabled={busy} onClick={() => void run("resume")}>Retry pending cleanup</Button>}
        </> : <Button color="error" variant="contained" disabled={busy || !ready || !progress} onClick={() => void run("start")}>{pending ? "Retry same revocation" : "Revoke on all Machines"}</Button>}
      </Stack>}>
      <Stack spacing={1.5}>
        <Typography variant="body2">This permanently disables this Cowboy account, stops its sessions and removes its native logins on every Machine, including Machines that reconnect later. The disabled account stays in history so cleanup can be tracked. Other subscriptions are unaffected. Your Provider subscription and billing continue.</Typography>
        {error && <Alert severity="warning">{error}</Alert>}
        {completed && <Alert severity="success">Cowboy cleanup is confirmed for every target. This account remains disabled.</Alert>}
        {started && !completed && <Typography role="status">Cleanup is pending. Offline Machines and unconfirmed process exits stay in the list until confirmed.</Typography>}
        <Stack sx={{ maxHeight: "35vh", overflowY: "auto" }}>
          {progress?.completion?.missing.map((item, index) => <Typography variant="body2" key={`${item.kind}:${item.target}:${index}`} sx={{ overflowWrap: "anywhere" }}>{item.kind.replaceAll("_", " ")} · {item.target}</Typography>)}
        </Stack>
      </Stack>
    </ConfirmSheet>
  </>;
}
