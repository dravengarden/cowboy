/** The only way a sign-in dialog shows its verification link: in the
 * account's ordinary Machine browser on the login's Machine, so every
 * request of the Provider page leaves from that Machine (owner decision
 * 2026-11-17). It opens (or attaches to) that tab browser and navigates it
 * once; the person completes the page and copies anything it shows back
 * into the dialog themselves. The browser never reads or fills anything of
 * the login. When the browser cannot be offered, the entry says why and
 * never falls back to a browser of this device. */
import { Button, Stack, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { productSessionSignal } from "../productSessionEnd";
import { productSyncPrincipal } from "../productSyncIdentity";
import { browserErrorText, type MachineBrowserApi, machineBrowserApi } from "./api";
import { linkBrowserOffered } from "./entryRules";
import { MachineBrowserSheet } from "./MachineBrowserSheet";

export function LinkBrowserEntry({
  owner,
  dataset,
  account,
  label,
  machine,
  url,
  disabled,
  api = machineBrowserApi,
}: {
  owner: string;
  dataset: string;
  account: string;
  label: string;
  machine: string;
  url: string;
  disabled: boolean;
  api?: MachineBrowserApi;
}): React.JSX.Element | null {
  /** `undefined` while the browser read is pending. */
  const [offered, setOffered] = useState<boolean>();
  const [expectedBrowser, setExpectedBrowser] = useState<string>();
  const [reason, setReason] = useState("");
  const [open, setOpen] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    const abort = new AbortController();
    setOffered(undefined);
    setExpectedBrowser(undefined);
    setReason("");
    api
      .read(
        account,
        AbortSignal.any([abort.signal, productSessionSignal(), AbortSignal.timeout(15_000)]),
        machine,
      )
      .then(
        (read) => {
          if (!abort.signal.aborted && productSyncPrincipal() === owner) {
            const ok = linkBrowserOffered(read, machine);
            setOffered(ok);
            setExpectedBrowser(ok ? read.browser_identity : undefined);
            if (!ok) setReason(read.admission ? browserErrorText(read.admission) : `This account’s Machine browser cannot run on ${machine} now.`);
          }
        },
        () => {
          if (!abort.signal.aborted) {
            setOffered(false);
            setExpectedBrowser(undefined);
            setReason("Could not read this account’s Machine browser.");
          }
        },
      );
    return () => abort.abort();
  }, [account, api, machine, owner, attempt]);
  if (!offered) {
    if (offered === undefined) {
      return <Typography variant="caption" color="text.secondary">Checking this account’s Machine browser…</Typography>;
    }
    return (
      <Stack spacing={0.5} data-link-browser-unavailable="">
        <Typography variant="caption" color="error">
          {reason} The sign-in page opens only in the Machine browser, never on this device.
          Enable or free the browser, then check again.
        </Typography>
        <Button size="small" sx={{ alignSelf: "flex-start" }} disabled={disabled} onClick={() => setAttempt((n) => n + 1)}>
          Check again
        </Button>
      </Stack>
    );
  }
  return (
    <>
      <Button variant="contained" disabled={disabled} onClick={() => setOpen(true)}>
        Open sign-in in this account’s Machine browser
      </Button>
      {open && (
        <MachineBrowserSheet
          key={`${account}:${machine}`}
          open
          onClose={() => setOpen(false)}
          owner={owner}
          dataset={dataset}
          account={{ id: account, label, browserEnabled: true }}
          link={{ url, machine, expectedBrowser }}
        />
      )}
    </>
  );
}
