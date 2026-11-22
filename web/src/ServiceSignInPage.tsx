/** The page step of a Service sign-in: where its verification link may
 * open. See `serviceSignInBrowser` for the rule: a retired card's page opens
 * only in its Machine browser (or not at all); other Providers keep the
 * device page. */
import { Alert, Button, Typography, Stack } from "@mui/material";
import type { useReliableTouchTap } from "./useReliableTouchTap";
import { LinkBrowserEntry } from "./machineBrowser/LinkBrowserEntry";
import { ProviderAccountSwitch } from "./ProviderAccountSwitch";
import type { ProviderAuthenticationFlow, ProviderLoginEvent } from "./providerAuthenticationOwner";
import { resolveProviderAuthenticationPresentation } from "@cowboy/provider-ui";
import { productSyncPrincipal } from "./productSyncIdentity";
import { type ServiceSignInBrowser, serviceSignInBrowser } from "./serviceSignInBrowser";
import { useSubscriptionAccounts } from "./subscriptionAccounts";

/** Where the open Service sign-in of `provider` may show its link. */
export function useServiceSignInBrowser(provider: string | undefined): ServiceSignInBrowser | undefined {
  const { data } = useSubscriptionAccounts();
  return provider === undefined ? undefined : serviceSignInBrowser(data, productSyncPrincipal(), provider);
}

export function ServiceSignInPage({
  browser,
  flow,
  challenge,
  busy,
  externalAction,
  pageTap,
  onCopyLink,
  onOpen,
  notice,
}: {
  browser: ServiceSignInBrowser | undefined;
  flow: ProviderAuthenticationFlow;
  challenge: Extract<ProviderLoginEvent, { event: "login_challenge" }>;
  busy: boolean;
  externalAction: string | undefined;
  pageTap: ReturnType<typeof useReliableTouchTap<HTMLButtonElement>>;
  onCopyLink: () => void;
  onOpen: () => void;
  notice: string;
}): React.JSX.Element {
  return (
    <>
      {browser?.kind === "machine"
        ? (
          <Stack spacing={1} data-service-sign-in-browser>
            <Typography variant="caption" color="text.secondary">
              The sign-in page opens only in this account’s
              Machine browser, so the Provider sees that
              Machine’s network and browser, never this device.
              Copy anything the page shows with the browser’s
              clipboard, then return here.
            </Typography>
            {flow.requestId && flow.machineId
              ? (
                <LinkBrowserEntry
                  owner={browser.owner}
                  dataset={browser.dataset}
                  account={browser.account}
                  label={flow.credentialTitle}
                  machine={flow.machineId}
                  url={challenge.verification_url}
                  disabled={busy}
                />
              )
              : (
                <Alert severity="warning">
                  This sign-in did not report its Machine. Close
                  it and start again.
                </Alert>
              )}
          </Stack>
        )
        : browser?.kind === "device"
        ? (
          <>
            <Typography variant="caption" color="text.secondary">
              The sign-in page opens on this device and uses its network
              connection. Choosing a remote Agent does not route browser
              traffic through that Machine.
            </Typography>
            <Button
              variant="contained"
              disabled={busy}
              {...pageTap}
            >
              {challenge.user_code
                ? "Copy code & open sign-in"
                : externalAction}
            </Button>
            <Typography variant="caption" color="text.secondary">
              {challenge.user_code
                ? "Cowboy copies the device code before opening the Provider page. Paste it only if the page does not fill it automatically. Close the browser to return here; Cowboy will keep waiting securely."
                : "After completing the Provider page, return to Cowboy. This dialog will keep waiting securely."}
            </Typography>
            {resolveProviderAuthenticationPresentation(
                  flow.provider.manifest.authentication,
                ) === "account" && (
              <ProviderAccountSwitch
                key={flow.requestId}
                disabled={busy}
                requiresCode={Boolean(challenge.input_required)}
                onCopy={onCopyLink}
                onOpen={onOpen}
                notice={notice}
              />
            )}
          </>
        )
        : browser?.kind === "refused"
        ? (
          <Alert severity="info" data-service-sign-in-refused>
            This legacy default account is retired, and its
            sign-in page never opens on this device. Add a named
            account in Settings → Subscriptions instead.
          </Alert>
        )
        : (
          <Typography variant="caption" color="text.secondary">
            Checking where this sign-in page may open…
          </Typography>
        )}
      {notice
        ? (
          <Alert
            severity={notice.startsWith(
                "Could not",
              )
              ? "warning"
              : "info"}
          >
            {notice}
          </Alert>
        )
        : null}
    </>
  );
}
