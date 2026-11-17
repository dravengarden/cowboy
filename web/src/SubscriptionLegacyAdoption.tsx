import { Alert, Button, Stack, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { productSessionSignal } from "./productSessionEnd";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import {
  type AdoptionAttempt,
  adoptionCard,
  adoptionErrorText,
  validAdoptionAttempt,
} from "./legacyAdoption";

/** "Upgrade to a named account": the legacy card becomes a named account with
 * the same label, browser and default, without signing in again. The
 * Service moves the credential into the account's vault for every Machine
 * and signs the legacy account out everywhere. Credentials never reach this
 * page. */
export function SubscriptionLegacyAdoption({
  snapshot,
  provider,
  owner,
  disabled,
  onChange,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  provider: string;
  owner: string | undefined;
  disabled: boolean;
  onChange: () => void;
}): React.JSX.Element | null {
  const card = adoptionCard(snapshot, provider);
  const key = `cowboy:legacy-adoption:${owner ?? "unbound"}:${card ?? ""}`;
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );
  if (!card || !owner) return null;
  const saved = (): AdoptionAttempt | undefined => {
    try {
      const value: unknown = JSON.parse(sessionStorage.getItem(key) ?? "null");
      return validAdoptionAttempt(value) ? value : undefined;
    } catch {
      return undefined;
    }
  };
  async function run(attempt: AdoptionAttempt) {
    sessionStorage.setItem(key, JSON.stringify(attempt));
    setBusy(true);
    setError("");
    try {
      const response = await fetch(
        `/api/provider-accounts/legacy/${encodeURIComponent(card!)}/adopt`,
        {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          cache: "no-store",
          signal: productSessionSignal(),
          body: JSON.stringify(attempt),
        },
      );
      const body = (await response.json().catch(() => undefined)) as
        | { status?: string; error?: { code?: string } }
        | undefined;
      if (response.ok && body?.status === "completed") {
        sessionStorage.removeItem(key);
        setOpen(false);
        onChange();
        return;
      }
      // A refusal before any effect has nothing to resume.
      if (response.status === 409) sessionStorage.removeItem(key);
      setError(adoptionErrorText(body?.error?.code));
    } catch {
      setError("The upgrade is not confirmed. Retry resumes it.");
    } finally {
      if (alive.current) setBusy(false);
    }
  }
  const pending = saved();
  const resumable = Boolean(pending) || snapshot?.legacy_adoption?.[card]?.state === "resumable";
  return (
    <Stack spacing={1} data-legacy-adoption={card}>
      {!open && !resumable && (
        <Button disabled={disabled || busy} onClick={() => setOpen(true)}>
          Upgrade to a named account
        </Button>
      )}
      {(open || resumable) && (
        <Stack spacing={1}>
          <Typography variant="caption" color="text.secondary">
            Moves this sign-in into a named account that works on every Machine, keeping its label,
            browser and default. No new sign-in is needed. The existing account is then signed out
            everywhere, including its open sessions.
          </Typography>
          <Stack direction="row" spacing={1}>
            <Button
              variant="contained"
              disabled={disabled || busy}
              onClick={() => void run(pending ?? { operation: crypto.randomUUID() })}
            >
              {resumable ? "Resume upgrade" : "Upgrade"}
            </Button>
            {!resumable && (
              <Button disabled={busy} onClick={() => setOpen(false)}>
                Cancel
              </Button>
            )}
          </Stack>
        </Stack>
      )}
      {busy && (
        <Typography variant="caption" color="text.secondary">
          Upgrading…
        </Typography>
      )}
      {error && <Alert severity="warning">{error}</Alert>}
    </Stack>
  );
}
