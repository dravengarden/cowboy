import { corePasskeysEnabled } from "./coreSecurity";
import { useProductAuth } from "./ProductAuthGate";
import { ProductPasskeysPanel } from "./ProductPasskeysPanel";

/** Core policy, not a Plugin slot/label/native claim, owns account security. */
export function ProductAccountSecurity(): React.JSX.Element | null {
  const { passkeys } = useProductAuth();
  return corePasskeysEnabled(passkeys) ? <ProductPasskeysPanel /> : null;
}
