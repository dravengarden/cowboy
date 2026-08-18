import { Button } from "@mui/material";
import { useState } from "react";
import { useProductAuth } from "./ProductAuthGate";

/** Minimal product sign-out. Lives in auth/ so Desktop/Mobile can call it
 *  without importing store.ts. */
export function ProductAccountMenu(): React.JSX.Element {
  const { me, signOut } = useProductAuth();
  const [busy, setBusy] = useState(false);
  return (
    <Button
      color="inherit"
      disabled={busy}
      onClick={() => {
        if (busy) return;
        setBusy(true);
        void signOut().finally(() => setBusy(false));
      }}
    >
      Sign out {me.account}
    </Button>
  );
}
