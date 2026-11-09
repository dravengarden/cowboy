import { Button, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { legacyBrowserOfProvider } from "./machineBrowser/entryRules";
import { MachineBrowserSheet } from "./machineBrowser/MachineBrowserSheet";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

/** The account whose browser the controls manage: a named account, or a
 * legacy default usage card through its own browser key. */
interface BrowserActionsAccount {
  id: string;
  label: string;
  browser_enabled?: boolean | undefined;
  revoking?: boolean | undefined;
}

/** Account-card controls for the account's Machine browser: open its panel
 * and toggle the per-account enablement (`set_browser_enabled`, or
 * `set_legacy_browser_enabled` for a legacy card), which the card's owner
 * journals like every other registry write. */
export function SubscriptionBrowserActions({
  account,
  owner,
  dataset,
  disabled,
  onSetBrowserEnabled,
}: {
  account: BrowserActionsAccount;
  owner: string;
  dataset: string;
  disabled: boolean;
  onSetBrowserEnabled: (enabled: boolean) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const enabled = account.browser_enabled === true;
  return (
    <>
      <Button disabled={Boolean(account.revoking)} onClick={() => setOpen(true)}>
        Browser…
      </Button>
      <Button
        disabled={disabled || Boolean(account.revoking)}
        onClick={() => onSetBrowserEnabled(!enabled)}
      >
        {enabled ? "Disable browser" : "Enable browser"}
      </Button>
      {open && (
        <MachineBrowserSheet
          key={account.id}
          open
          onClose={() => setOpen(false)}
          owner={owner}
          dataset={dataset}
          account={{ id: account.id, label: account.label, browserEnabled: enabled }}
          onEnableBrowser={() => onSetBrowserEnabled(true)}
          enabling={disabled}
        />
      )}
    </>
  );
}

/** The browser controls on the legacy (Service account) row of `provider`:
 * the browser of the usage card that shows this Provider's default account.
 * A card collected for several Providers is controlled from its first
 * Provider's row only; it is one card with one browser. Off by default. */
export function LegacyBrowserActions({
  snapshot,
  provider,
  label,
  owner,
  disabled,
  onSetBrowserEnabled,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  provider: string;
  label: string;
  owner: string | undefined;
  disabled: boolean;
  onSetBrowserEnabled: (card: string, enabled: boolean) => void;
}): React.JSX.Element | null {
  const browser = legacyBrowserOfProvider(snapshot, provider);
  if (!browser || !owner || !snapshot) return null;
  return (
    <Stack spacing={0.25} data-legacy-browser={browser.card}>
      <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap>
        <SubscriptionBrowserActions
          account={{ id: browser.account_id, label, browser_enabled: browser.browser_enabled }}
          owner={owner}
          dataset={snapshot.dataset.dataset_id}
          disabled={disabled}
          onSetBrowserEnabled={(enabled) => onSetBrowserEnabled(browser.card, enabled)}
        />
      </Stack>
      <Typography variant="caption" color="text.secondary">
        Machine browser of this usage card
        {browser.providers.length > 1 ? ` (shared card of ${browser.providers.join(", ")})` : ""}
        {snapshot.machine_browser !== "enabled" ? " · not enabled on this Service" : ""}
      </Typography>
    </Stack>
  );
}
