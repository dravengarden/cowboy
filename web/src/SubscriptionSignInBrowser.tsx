import { Button } from "@mui/material";
import { useEffect, useState } from "react";
import { machineBrowserApi } from "./machineBrowser/api";
import { MachineBrowserSheet } from "./machineBrowser/MachineBrowserSheet";
import { productSessionSignal } from "./productSessionEnd";
import { productSyncPrincipal } from "./productSyncIdentity";
import type { LoginScope } from "./subscriptionLoginController";

/** Sign-in dialog entry to the account's Machine browser. Shown only when the
 * Controller admits a browser for this account; opening binds the browser to
 * this native login. The remote clipboard fills the code field only through
 * the explicit action in the sheet, never automatically. */
export function SubscriptionSignInBrowser({
  scope,
  label,
  request,
  disabled,
  onCode,
}: {
  scope: LoginScope;
  label: string;
  request: string;
  disabled: boolean;
  onCode: (code: string) => void;
}): React.JSX.Element | null {
  const [admitted, setAdmitted] = useState(false);
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const abort = new AbortController();
    setAdmitted(false);
    machineBrowserApi
      .read(
        scope.account,
        AbortSignal.any([abort.signal, productSessionSignal(), AbortSignal.timeout(15_000)]),
      )
      .then(
        (read) => {
          if (!abort.signal.aborted && productSyncPrincipal() === scope.owner)
            setAdmitted(read.admission === null);
        },
        () => undefined,
      );
    return () => abort.abort();
  }, [scope.account, scope.owner]);
  if (!admitted) return null;
  return (
    <>
      <Button disabled={disabled} onClick={() => setOpen(true)}>
        Sign in on the Machine’s browser
      </Button>
      {open && (
        <MachineBrowserSheet
          open
          onClose={() => setOpen(false)}
          owner={scope.owner}
          dataset={scope.dataset}
          account={{ id: scope.account, label, browserEnabled: true }}
          signInRequest={request}
          autoOpen
          onUseCode={(code) => {
            onCode(code);
            setOpen(false);
          }}
        />
      )}
    </>
  );
}
