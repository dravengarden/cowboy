import { Box, CircularProgress } from "@mui/material";
import { useEffect, useState } from "react";
import { type ConnectionStore, useAutoUpdate } from "@cowboy/app-shell";
import { canApplyUpdateNow } from "../store";
import {
  fetchReadyCowboyVersion,
  mobileUpdateBannerLabel,
  type MobileUpdatePhase,
} from "./mobileUpdateVersion";

/**
 * The phone installs a deployed build by itself. A foreground service-worker
 * check may discover a deploy while the user is reading or composing, so the
 * page waits for a real pause instead of asking for a tap: no composer text,
 * no running turn, nothing in flight, and a full minute of foreground since
 * the app was last resumed (`useAutoUpdate` owns that policy). Until then the
 * bar only narrates, and an update that never finds its pause is applied by
 * the next launch anyway.
 *
 * Connectivity itself is not a banner here: the sync status pill
 * (`MobileSyncPill`) presents outages, reconnects and queued work.
 */

// A resumed PWA restores its frozen page. Reloading in the seconds after
// someone opened the app reads as a crash, so the update waits out a minute of
// uninterrupted foreground first.
const MOBILE_UPDATE_DWELL_MS = 60_000;

export function MobileConnectionBanner(
  { store }: { readonly store: ConnectionStore },
): React.JSX.Element | null {
  const rawBanner = store.useConnectionBanner();
  const banner = rawBanner?.kind === "update" ? rawBanner : undefined;
  const isUpdate = banner?.kind === "update";
  const update = useAutoUpdate(store, {
    canApplyUpdate: canApplyUpdateNow,
    minVisibleMs: MOBILE_UPDATE_DWELL_MS,
  });
  const [readyVersion, setReadyVersion] = useState<string>();
  useEffect(() => {
    if (!isUpdate) {
      setReadyVersion(undefined);
      return undefined;
    }
    let cancelled = false;
    void (async (): Promise<void> => {
      const registration = "serviceWorker" in navigator
        ? await navigator.serviceWorker.getRegistration()
        : undefined;
      const version = await fetchReadyCowboyVersion(
        undefined,
        registration?.waiting?.scriptURL,
      );
      if (!cancelled) setReadyVersion(version);
    })();
    return (): void => {
      cancelled = true;
    };
  }, [isUpdate]);
  if (!banner) return null;
  const phase: MobileUpdatePhase = update.applying
    ? { kind: "applying" }
    : update.failed
    ? { kind: "failed" }
    : update.held
    ? { kind: "held" }
    : { kind: "counting", secs: update.secs };
  const label = mobileUpdateBannerLabel(readyVersion, phase);

  return (
    <Box
      role="status"
      aria-live="polite"
      sx={{
        position: "absolute",
        top: 0,
        left: 0,
        right: 0,
        width: "100%",
        maxWidth: "100%",
        boxSizing: "border-box",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        flexWrap: "wrap",
        gap: 1,
        px: 2,
        py: 0.75,
        pt: "calc(var(--cowboy-system-top-clearance) + 6px)",
        bgcolor: "info.main",
        color: "info.contrastText",
        fontSize: "0.8125rem",
        fontWeight: 600,
        zIndex: (theme) => theme.zIndex.tooltip + 1,
      }}
    >
      {update.applying && <CircularProgress size={14} color="inherit" thickness={5} />}
      <span>{label}</span>
    </Box>
  );
}
