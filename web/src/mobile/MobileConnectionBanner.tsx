import { ButtonBase, CircularProgress } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import {
  type ConnectionStore,
  updateFillShare,
  updateFillSx,
  useAutoUpdate,
} from "@cowboy/app-shell";
import { confirmationHaptic } from "../haptic";
import { canApplyUpdateNow } from "../store";
import {
  fetchReadyCowboyVersion,
  mobileUpdateBannerLabel,
  type MobileUpdatePhase,
} from "./mobileUpdateVersion";

/**
 * The phone installs a deployed build by itself, and the bar that says so is
 * also the way to have it now.
 *
 * The bits are fetched the instant a deploy is detected, and the bar fills with
 * that download, so the press it offers is honest: by the time it reads
 * "Reload", the new build is wholly on the device and the swap needs no network
 * at all. Pressing earlier is allowed and means the same thing — take it as
 * soon as it lands — which is why the control is never disabled. A disabled
 * control on a phone reads as broken, and it would make the user watch a
 * progress bar for permission to say what they already decided.
 *
 * Nothing waits on that press. Left alone, the page still reloads on the first
 * real pause: no composer text, no running turn, nothing in flight, and a full
 * minute of foreground since the app was last resumed (`useAutoUpdate` owns
 * that policy), and an update that never finds its pause is applied by the next
 * launch anyway.
 *
 * Connectivity itself is not a banner here: the sync status pill
 * (`MobileSyncPill`) presents outages, reconnects and queued work.
 */

// A resumed PWA restores its frozen page. Reloading in the seconds after
// someone opened the app reads as a crash, so the automatic update waits out a
// minute of uninterrupted foreground first. A press is exempt: the user is
// looking at the control they just touched.
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
  // One tick when the download lands, so the user can look away during it and
  // still know the press is now instant.
  const arrived = useRef(false);
  useEffect(() => {
    if (update.phase !== "ready") {
      if (update.phase === "downloading") arrived.current = false;
      return;
    }
    if (arrived.current || !update.streamed) return;
    arrived.current = true;
    if (globalThis.document.visibilityState === "visible") confirmationHaptic();
  }, [update.phase, update.streamed]);
  if (!banner) return null;
  const phase: MobileUpdatePhase = update.phase === "reloading"
    ? { kind: "reloading" }
    : update.phase === "failed"
    ? { kind: "failed", requested: update.requested }
    : update.phase === "downloading"
    ? { kind: "downloading", progress: update.progress, requested: update.requested }
    : { kind: "ready", secs: update.held ? undefined : update.secs };
  const label = mobileUpdateBannerLabel(readyVersion, phase);

  return (
    <ButtonBase
      aria-live="polite"
      aria-label={label}
      // No ripple: the bar is its own progress fill, and a ripple would lay a
      // second animated layer over the pager chrome. The press answers in the
      // label, which is the feedback that means anything here.
      disableRipple
      disabled={update.phase === "reloading"}
      onClick={update.requestUpdate}
      sx={(theme) => ({
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
        color: "info.contrastText",
        fontSize: "0.8125rem",
        fontWeight: 600,
        zIndex: theme.zIndex.tooltip + 1,
        // The bar is its own progress bar: full width, so the press needs no
        // aim, and paint-only, so it never becomes a second moving layer over
        // the pager (docs/mobile-spatial-presentation.md §2.1).
        ...updateFillSx(
          theme.palette.info.main,
          theme.palette.info.dark,
          updateFillShare(update.phase, update.progress),
          update.streamed,
        ),
      })}
    >
      {(update.phase === "reloading" ||
        (update.phase === "downloading" && update.progress === undefined)) && (
        <CircularProgress size={14} color="inherit" thickness={5} />
      )}
      <span>{label}</span>
    </ButtonBase>
  );
}
