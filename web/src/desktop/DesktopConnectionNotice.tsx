import { useEffect, useRef, useState } from "react";
import { retrySyncNow, useSyncStatus } from "../store";
import {
  presentedSyncPhase,
  SYNC_PRESENTATION_DEBOUNCE_MS,
  type SyncPhase,
} from "../syncStatus";
import { connectionNotice } from "./connectionNotice";
import { ConnectionNoticeStrip } from "./ConnectionNoticeStrip";

/**
 * Strip above the Desktop composer while the Cowboy server cannot be used.
 * It waits out the presentation debounce so a reconnect blip never flashes,
 * says whether the device or the server is the problem, and confirms the
 * recovery briefly before it leaves.
 */
export function DesktopConnectionNotice(): React.JSX.Element | null {
  const status = useSyncStatus();
  const [now, setNow] = useState(() => Date.now());
  const shownRef = useRef<SyncPhase | null>(null);
  const recoveredAtRef = useRef<number | undefined>(undefined);
  const wasLiveRef = useRef(status.phase === "live");

  if (
    status.phase === "live" && !wasLiveRef.current && shownRef.current !== null
  ) {
    recoveredAtRef.current = Date.now();
  }
  wasLiveRef.current = status.phase === "live";
  const presented = presentedSyncPhase(
    status,
    shownRef.current,
    recoveredAtRef.current,
    now,
  );
  shownRef.current = presented === null || presented === "recovered"
    ? null
    : presented;

  // Tick for the debounce, the retry countdown and the recovery flash.
  const ticking = status.phase !== "live" || presented !== null;
  useEffect(() => {
    if (!ticking) return undefined;
    setNow(Date.now());
    const timer = globalThis.setInterval(() => setNow(Date.now()), 1000);
    return () => globalThis.clearInterval(timer);
  }, [ticking]);
  useEffect(() => {
    if (status.phase === "live") return undefined;
    const timer = globalThis.setTimeout(
      () => setNow(Date.now()),
      SYNC_PRESENTATION_DEBOUNCE_MS + 16,
    );
    return () => globalThis.clearTimeout(timer);
  }, [status.phase, status.since]);

  const notice = connectionNotice(status, presented, now);
  if (notice === null) return null;
  return (
    <ConnectionNoticeStrip
      notice={notice}
      onRetry={(): void => retrySyncNow()}
    />
  );
}
