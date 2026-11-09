/** The Browser entry of a Provider-account usage card: the same panel the
 * account management card opens, for exactly that card's ordinary tab
 * browser. A named account opens its own browser; a legacy default card
 * (the Service-collected default account of a usage provider) opens the
 * card's own browser key, or the owner-defined shared profile the card was
 * put into (shown as "Shared · <group>"). It never enables
 * anything; a closed gate or a disabled browser says why and links to
 * account management. */
import { OpenInBrowserRounded } from "@mui/icons-material";
import { Button, Link, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { openAppSettings } from "../appSettings";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import { accountBrowserEntry } from "./entryRules";
import { MachineBrowserSheet } from "./MachineBrowserSheet";
import { sharedBrowser } from "./shareRules";

export function AccountBrowserEntry({
  snapshot,
  account: id,
  label,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  /** The usage card id: a named account id or a legacy card's usage provider. */
  account: string;
  /** The card title, shown by the panel of a legacy card. */
  label?: string | undefined;
}): React.JSX.Element | null {
  const [open, setOpen] = useState(false);
  const entry = accountBrowserEntry(snapshot, id, label);
  if (entry.kind === "none" || !snapshot) return null;
  const { account } = entry;
  const shared = sharedBrowser(snapshot, account.id);
  return (
    <Stack spacing={0.25} data-account-browser-entry={account.id}>
      <Button
        size="small"
        startIcon={<OpenInBrowserRounded fontSize="small" />}
        disabled={entry.kind !== "ready"}
        onClick={() => setOpen(true)}
        sx={{ alignSelf: "flex-start", minHeight: 36 }}
      >
        Browser
      </Button>
      {shared && (
        <Typography variant="caption" color="text.secondary" data-browser-shared={shared.label}>
          Shared · {shared.label}
        </Typography>
      )}
      {entry.kind === "disabled" && (
        <Typography variant="caption" color="text.secondary">
          {entry.reason}
          {entry.manage && (
            <>
              {" "}
              <Link
                component="button"
                variant="caption"
                onClick={() => openAppSettings({ tab: "providers" })}
              >
                Manage accounts
              </Link>
            </>
          )}
        </Typography>
      )}
      {open && (
        <MachineBrowserSheet
          key={account.id}
          open
          onClose={() => setOpen(false)}
          owner={snapshot.dataset.user_id}
          dataset={snapshot.dataset.dataset_id}
          account={{
            id: account.id,
            label: account.label,
            browserEnabled: account.browser_enabled,
            shared,
          }}
        />
      )}
    </Stack>
  );
}
