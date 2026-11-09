/** Optional convenience of a sign-in dialog: show its verification link in
 * the account's ordinary Machine browser on the login's Machine. It opens
 * (or attaches to) that tab browser and navigates it once; the person
 * completes the page and copies anything it shows back into the dialog
 * themselves. The login never depends on this entry, and the browser never
 * reads or fills anything of the login. */
import { Button } from "@mui/material";
import { useEffect, useState } from "react";
import { productSessionSignal } from "../productSessionEnd";
import { productSyncPrincipal } from "../productSyncIdentity";
import { type MachineBrowserApi, machineBrowserApi } from "./api";
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
  const [offered, setOffered] = useState(false);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    setOffered(false);
    api
      .read(
        account,
        AbortSignal.any([abort.signal, productSessionSignal(), AbortSignal.timeout(15_000)]),
        machine,
      )
      .then(
        (read) => {
          if (!abort.signal.aborted && productSyncPrincipal() === owner) {
            setOffered(linkBrowserOffered(read, machine));
          }
        },
        () => undefined,
      );
    return () => abort.abort();
  }, [account, api, machine, owner]);
  if (!offered) return null;
  return (
    <>
      <Button disabled={disabled} onClick={() => setOpen(true)}>
        Open in this account’s Machine browser
      </Button>
      {open && (
        <MachineBrowserSheet
          key={`${account}:${machine}`}
          open
          onClose={() => setOpen(false)}
          owner={owner}
          dataset={dataset}
          account={{ id: account, label, browserEnabled: true }}
          link={{ url, machine }}
        />
      )}
    </>
  );
}
