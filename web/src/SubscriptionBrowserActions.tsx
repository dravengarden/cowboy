import { GroupsRounded } from "@mui/icons-material";
import { Button, Typography } from "@mui/material";
import { useState } from "react";
import { MachineBrowserSheet } from "./machineBrowser/MachineBrowserSheet";
import { type SharedBrowser, sharedCardLabel } from "./machineBrowser/shareRules";

/** The named account whose browser the controls manage. */
interface BrowserActionsAccount {
  id: string;
  label: string;
  browser_enabled?: boolean | undefined;
  revoking?: boolean | undefined;
}

/** Account-card controls for the account's Machine browser: open its panel
 * and toggle the per-account enablement (`set_browser_enabled`), which the
 * card's owner journals like every other registry write. */
export function SubscriptionBrowserActions({
  account,
  owner,
  dataset,
  disabled,
  shared,
  onSetBrowserEnabled,
}: {
  account: BrowserActionsAccount;
  owner: string;
  dataset: string;
  disabled: boolean;
  /** The shared browser profile the account uses, if any. */
  shared?: SharedBrowser | undefined;
  onSetBrowserEnabled: (enabled: boolean) => void;
}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  const enabled = account.browser_enabled === true;
  return (
    <>
      <Button
        disabled={Boolean(account.revoking)}
        color={shared ? "secondary" : "primary"}
        startIcon={shared ? <GroupsRounded fontSize="small" /> : undefined}
        aria-label={shared ? `Shared browser: ${shared.label}` : "Browser"}
        onClick={() => setOpen(true)}
      >
        {shared ? "Shared browser…" : "Browser…"}
      </Button>
      {shared && (
        <Typography
          variant="caption"
          color="secondary.main"
          data-browser-shared={shared.label}
          sx={{ alignSelf: "center" }}
        >
          {sharedCardLabel(shared)}
        </Typography>
      )}
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
