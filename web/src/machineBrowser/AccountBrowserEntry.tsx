/** The Browser entry of a Provider-account usage card: the same panel the
 * account management card opens, for exactly that card's ordinary tab
 * browser. A named account opens its own browser; a legacy default card
 * (the Service-collected default account of a usage provider) opens the
 * card's own browser key, or the owner-defined shared profile the card was
 * put into (shown as "Group <name> · with <members>"). It never enables
 * anything; a closed gate or a disabled browser says why and links to
 * account management. */
import { GroupsRounded, OpenInBrowserRounded } from "@mui/icons-material";
import { Button, Link, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { openAppSettings } from "../appSettings";
import type { SubscriptionSnapshot } from "../subscriptionAccounts";
import { accountBrowserEntry } from "./entryRules";
import { MachineBrowserSheet } from "./MachineBrowserSheet";
import { inventoryBrowserIdentity, sharedBrowser, sharedCardLabel } from "./shareRules";

export function AccountBrowserEntry({
  snapshot,
  account: id,
  label,
  pageLink,
}: {
  snapshot: SubscriptionSnapshot | undefined;
  /** The usage card id: a named account id or a legacy card's usage provider. */
  account: string;
  /** The card title, shown by the panel of a legacy card. */
  label?: string | undefined;
  /** Optional vendor page (for example a web-only usage/reset page) opened
   * directly in this account's Machine browser. */
  pageLink?: { url: string; label: string } | undefined;
}): React.JSX.Element | null {
  const [openTarget, setOpenTarget] = useState<{
    url?: string;
    expectedBrowser?: string | undefined;
  } | null>(null);
  const entry = accountBrowserEntry(snapshot, id, label);
  if (entry.kind === "none" || !snapshot) return null;
  const { account } = entry;
  const shared = sharedBrowser(snapshot, account.id);
  const expectedBrowser = inventoryBrowserIdentity(snapshot, shared);
  return (
    <Stack spacing={0.25} data-account-browser-entry={account.id}>
      <Stack direction="row" spacing={0.5} flexWrap="wrap" alignItems="center">
        <Button
          size="small"
          color={shared ? "secondary" : "primary"}
          startIcon={
            shared ? <GroupsRounded fontSize="small" /> : <OpenInBrowserRounded fontSize="small" />
          }
          aria-label={shared ? `Shared browser: ${shared.label}` : "Browser"}
          disabled={entry.kind !== "ready"}
          onClick={() => setOpenTarget({})}
          sx={{ alignSelf: "flex-start", minHeight: 36 }}
        >
          {shared ? "Shared browser" : "Browser"}
        </Button>
        {entry.kind === "ready" && pageLink && (
          <Button
            size="small"
            variant="outlined"
            color={shared ? "secondary" : "primary"}
            startIcon={<OpenInBrowserRounded fontSize="small" />}
            aria-label={
              shared
                ? `${pageLink.label} in shared browser: ${shared.label}`
                : `${pageLink.label} in browser`
            }
            data-account-browser-page-link={pageLink.url}
            onClick={() => setOpenTarget({ url: pageLink.url, expectedBrowser })}
            sx={{ alignSelf: "flex-start", minHeight: 36, textTransform: "none" }}
          >
            {pageLink.label}
          </Button>
        )}
      </Stack>
      {shared && (
        <Typography variant="caption" color="secondary.main" data-browser-shared={shared.label}>
          {sharedCardLabel(shared)}
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
      {openTarget !== null && (
        <MachineBrowserSheet
          key={`${account.id}:${openTarget.url ?? ""}`}
          open
          onClose={() => setOpenTarget(null)}
          owner={snapshot.dataset.user_id}
          dataset={snapshot.dataset.dataset_id}
          account={{
            id: account.id,
            label: account.label,
            browserEnabled: account.browser_enabled,
          }}
          link={
            openTarget.url
              ? { url: openTarget.url, expectedBrowser: openTarget.expectedBrowser }
              : undefined
          }
        />
      )}
    </Stack>
  );
}
