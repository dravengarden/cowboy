/** The Browser entry of a Provider-account usage card: the same panel the
 * account management card opens, for exactly that account's ordinary tab
 * browser. It never enables anything; a closed gate or a disabled browser
 * says why and links to account management. Cards that are not named
 * registry accounts (the Service-collected default account of a Provider)
 * get no entry: the browser routes accept named accounts only. */
import { OpenInBrowserRounded } from "@mui/icons-material";
import { Button, Link, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { openAppSettings } from "../appSettings";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import { accountBrowserEntry } from "./entryRules";
import { MachineBrowserSheet } from "./MachineBrowserSheet";

export function AccountBrowserEntry({
  snapshot,
  account: id,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  account: string;
}): React.JSX.Element | null {
  const [open, setOpen] = useState(false);
  const entry = accountBrowserEntry(snapshot, id);
  if (entry.kind === "none" || !snapshot) return null;
  const { account } = entry;
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
            browserEnabled: account.browser_enabled === true,
          }}
        />
      )}
    </Stack>
  );
}
