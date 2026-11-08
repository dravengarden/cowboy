import { Button } from "@mui/material";
import { useState } from "react";
import { MachineBrowserSheet } from "./machineBrowser/MachineBrowserSheet";
import type { SubscriptionAccount } from "./subscriptionAccounts";

/** Account-card controls for the account's Machine browser: open its panel
 * and toggle the per-account enablement (`set_browser_enabled`), which the
 * card's owner journals like every other registry write. */
export function SubscriptionBrowserActions({
  account,
  owner,
  dataset,
  disabled,
  onSetBrowserEnabled,
}: {
  account: SubscriptionAccount;
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
