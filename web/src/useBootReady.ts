import { useEffect } from "react";
import { signalBootReady } from "./bootSnapshot";

/** Tell the boot overlay that this view has painted what the user came for
 * (docs/offline-first-sync.md §Boot presentation). Idempotent. */
export function useBootReady(ready: boolean): void {
  useEffect(() => {
    if (ready) signalBootReady();
  }, [ready]);
}
