// ConnectionBanner — the unified connection / version banner.
//
// One floating top bar + one store policy for every app that lives behind a
// long-lived socket and ships its own build id. Ported verbatim (behavior +
// visual) from liveview's connectionStore.ts + ReconnectBanner.tsx; cowboy now
// shares it too, so the two apps behave identically.
//
// The store side is a FACTORY (createConnectionStore) returning a fresh,
// self-contained instance — each app holds its own singleton. The app's own
// socket layer drives the reconnect side (connectionReady / connectionLost); the
// version side is probed after each reconnect and whenever the tab returns to
// the foreground. The three states:
//   - red "down"          — reconnect has failed reconnectBannerThreshold times
//                           in a row (a blip that recovers on the first retry
//                           stays silent);
//   - green "reconnected" — the socket came back after a surfaced outage;
//                           auto-dismissed after reconnectedDismissMs;
//   - blue "update"       — a redeploy was detected; the floating overlay
//                           downloads the deployed build at once, fills with
//                           that progress, and reloads into it on its own —
//                           or the instant the bar is pressed.
//
// Only the banner is shared React state (read via useConnectionBanner); the
// socket itself stays in the app, which just reports open/close here and reads
// back the backoff delay.

import { Box, ButtonBase, CircularProgress } from "@mui/material";
import type { SxProps } from "@mui/material";
import type { Theme } from "@mui/material/styles";
import CheckIcon from "@mui/icons-material/Check";
import { type ReactNode, useCallback, useEffect, useState, useSyncExternalStore } from "react";
import {
  tickUpdateCountdown,
  updateAllowed,
  type UpdateCountdown,
  type UpdatePhase,
  updateReloadsNow,
} from "./update-policy.ts";
import {
  updateFillShare,
  updateFillSx,
  updatePercentLabel,
} from "./update-presentation.ts";

export type BannerKind = "down" | "reconnected" | "update";
export interface Banner {
  readonly kind: BannerKind;
}

export interface ConnectionStoreOptions {
  /** Build-id probe endpoint. liveview "/api/version", cowboy "/version". */
  readonly versionUrl: string;
  /** Surface the red banner once this many consecutive (re)connect cycles fail.
   *  Default 2 — a single dropped frame that recovers on the first retry stays
   *  silent; only a real outage raises the banner. */
  readonly reconnectBannerThreshold?: number;
  /** Cap the exponential backoff so a long outage doesn't hammer the server.
   *  Default 15000ms. */
  readonly reconnectBackoffMaxMs?: number;
  /** How long the green "reconnected" flash lingers before auto-dismissing.
   *  Default 4000ms. */
  readonly reconnectedDismissMs?: number;
}

export interface ConnectionStore {
  /** Call from the app's socket on a successful (re)open. Clears the failure
   *  count, flashes green if a red banner was up, then probes for a new build. */
  connectionReady(): void;
  /** Call from the app's socket on close. Raises the red banner once retries
   *  have failed past the threshold and returns the backoff delay (ms) the app
   *  should wait before its next attempt. */
  connectionLost(): number;
  /** A Service Worker with a fresh shell took control. Surface the same visible
   *  countdown used by build-id probes instead of reloading immediately. */
  updateAvailable(): void;
  /** Have the service worker cache the deployed shell and its boot assets,
   *  reporting each landed batch. The running build is untouched, so this may
   *  start the moment a deploy is detected. */
  downloadUpdate(onProgress: (done: number, total: number) => void): Promise<UpdateDownload>;
  /** Swap the running build for the downloaded one. */
  reloadIntoUpdate(downloaded: UpdateDownload): Promise<void>;
  /** Probe for a new build whenever the tab returns to the foreground. Returns a
   *  cleanup fn for the effect. */
  watchForegroundVersion(): () => void;
  /** useSyncExternalStore over this instance's banner. */
  useConnectionBanner(): Banner | undefined;
  /** The current known build id — for cache-busting fetches keyed on the build. */
  version(): string | undefined;
}

// Surface the red banner once this many consecutive (re)connect cycles fail.
const DEFAULT_RECONNECT_BANNER_THRESHOLD = 2;
// Cap the exponential backoff so a long outage doesn't hammer the server.
const DEFAULT_RECONNECT_BACKOFF_MAX_MS = 15_000;
// How long the green "reconnected" flash lingers before auto-dismissing.
const DEFAULT_RECONNECTED_DISMISS_MS = 4000;

const SHELL_REFRESH_TIMEOUT_MS = 30_000;

type ShellMessage = { ok?: boolean; type?: string; done?: number; total?: number };

/** Ask the controlling service worker for the deployed shell, reporting the
 *  boot-asset count as each batch lands. `undefined` means no worker controls
 *  this page (native WKWebView, first install), so nothing can be pre-fetched
 *  and the reload itself remains the download. */
function refreshShellThroughServiceWorker(
  onProgress: (done: number, total: number) => void,
): Promise<boolean | undefined> {
  const controller = globalThis.navigator?.serviceWorker?.controller;
  if (!controller || typeof MessageChannel === "undefined") return Promise.resolve(undefined);
  return new Promise((resolve) => {
    const channel = new MessageChannel();
    let timer = setTimeout(() => resolve(false), SHELL_REFRESH_TIMEOUT_MS);
    channel.port1.onmessage = (event: MessageEvent<ShellMessage>): void => {
      const message = event.data;
      clearTimeout(timer);
      if (message?.type === "progress") {
        // A download that is visibly advancing is slow, not stuck: the timeout
        // measures silence, not the whole transfer. A phone on a weak link
        // trickling in a large bundle must not be called a failure.
        timer = setTimeout(() => resolve(false), SHELL_REFRESH_TIMEOUT_MS);
        onProgress(message.done ?? 0, message.total ?? 0);
        return;
      }
      resolve(message?.ok === true);
    };
    try {
      // Ask for batches explicitly. An older worker ignores the flag and still
      // answers with its single {ok}, which this handler reads the same way, so
      // a new page never hangs on a service worker that predates progress.
      controller.postMessage({ type: "cowboy.refresh-shell", progress: true }, [channel.port2]);
    } catch {
      clearTimeout(timer);
      resolve(undefined);
    }
  });
}

/** How a pending update's bits arrived.
 *  - `ready`       — cached whole; the swap needs no network at all.
 *  - `unsupported` — no service worker controls this page, so the reload is
 *                    still the download and nothing may be promised about it.
 *  - `failed`      — they are not all here; this build keeps running. */
export type UpdateDownload = "ready" | "unsupported" | "failed";

// Fetch the deployed build without disturbing the running one. Started as soon
// as a deploy is detected: what interrupts someone is the reload, never the
// download, and paying for the bits early is what lets the swap be instant and
// survive a weak connection. Captures no per-instance state, so it lives at
// module scope.
async function downloadUpdate(
  onProgress: (done: number, total: number) => void,
): Promise<UpdateDownload> {
  const refreshed = await refreshShellThroughServiceWorker(onProgress);
  if (refreshed === undefined) return "unsupported";
  return refreshed ? "ready" : "failed";
}

// Swap the running build for the downloaded one. After a `ready` download the
// navigation is answered wholly from cache, so this cannot strand the page on a
// weak connection. Without a service worker the reload IS the download, so
// every cache is cleared first as before.
async function reloadIntoUpdate(downloaded: UpdateDownload): Promise<void> {
  if (downloaded !== "ready") {
    try {
      if ("caches" in globalThis) {
        const keys = await globalThis.caches.keys();
        await Promise.all(keys.map((k) => globalThis.caches.delete(k)));
      }
    } catch {
      // non-fatal — the reload still pulls fresh content-hashed assets.
    }
  }
  globalThis.location.reload();
}

// After a download that did not finish, Desktop waits this long before its
// countdown starts again.
const UPDATE_RETRY_MS = 60_000;

export function createConnectionStore(opts: ConnectionStoreOptions): ConnectionStore {
  const { versionUrl } = opts;
  const reconnectBannerThreshold = opts.reconnectBannerThreshold ?? DEFAULT_RECONNECT_BANNER_THRESHOLD;
  const reconnectBackoffMaxMs = opts.reconnectBackoffMaxMs ?? DEFAULT_RECONNECT_BACKOFF_MAX_MS;
  const reconnectedDismissMs = opts.reconnectedDismissMs ?? DEFAULT_RECONNECTED_DISMISS_MS;

  // ─── Per-instance closure state (was module-level in liveview's store) ─────
  let banner: Banner | undefined = undefined;
  const listeners = new Set<() => void>();
  // Consecutive failed (re)connect cycles; reset to 0 on a successful open.
  let attempts = 0;
  // Whether the current outage actually surfaced the red banner — so the reopen
  // only flashes green for outages the user was told about, not a sub-threshold
  // blip.
  let outageSurfaced = false;
  let reconnectedTimer: ReturnType<typeof setTimeout> | undefined = undefined;
  // The build id this tab loaded against; re-probed after each reconnect and on
  // foreground. A change means the server was redeployed under a now-stale tab.
  let knownVersion: string | undefined = undefined;

  function emit(): void {
    for (const l of listeners) {
      l();
    }
  }

  function setBanner(next: Banner | undefined): void {
    banner = next;
    emit();
  }

  async function probeVersion(): Promise<void> {
    let probed: string | undefined = undefined;
    try {
      const res = await globalThis.fetch(versionUrl, { cache: "no-store" });
      if (!res.ok) {
        return;
      }
      ({ version: probed } = (await res.json()) as { version: string });
    } catch {
      return; // network hiccup mid-probe; try again on the next trigger
    }
    if (knownVersion === undefined) {
      knownVersion = probed;
      return;
    }
    if (probed !== knownVersion) {
      setBanner({ kind: "update" });
    }
  }

  // Called by the app on a successful (re)open. Clears the failure count,
  // flashes green if a red banner was up, then probes for a new build first thing.
  function connectionReady(): void {
    const recovered = outageSurfaced;
    attempts = 0;
    outageSurfaced = false;
    // Recovered from a surfaced outage → flash green, but never stomp a sticky
    // blue update banner (it outranks everything). The async probe may replace
    // the green with blue moments later.
    if (recovered && banner?.kind !== "update") {
      setBanner({ kind: "reconnected" });
      if (reconnectedTimer) {
        clearTimeout(reconnectedTimer);
      }
      reconnectedTimer = setTimeout(() => {
        reconnectedTimer = undefined;
        // Only clear if still green — don't stomp an update banner the probe
        // raised in the meantime.
        if (banner?.kind === "reconnected") {
          setBanner(undefined);
        }
      }, reconnectedDismissMs);
    }
    void probeVersion();
  }

  // Called by the app on close. Raises the red banner once retries have failed
  // past the threshold (never stomping a sticky update banner) and returns the
  // backoff delay the app should wait before the next attempt.
  function connectionLost(): number {
    attempts += 1;
    if (attempts >= reconnectBannerThreshold && banner?.kind !== "update") {
      outageSurfaced = true;
      setBanner({ kind: "down" });
    }
    // Probe the build on EVERY drop, not just on a successful reconnect. A deploy
    // restarts the server, which is often WHY we just disconnected — and if the WS
    // reconnect then wedges, connectionReady (the only other probe trigger besides
    // tab-foreground) never fires, so the new build would otherwise stay invisible
    // and the tab sits on "reconnecting…" forever. Probing here means: once the
    // server is back as a new build, the next drop detects it → update banner →
    // auto-reload onto the fresh bundle. The fetch just fails (no-op) while the
    // server is still down mid-restart.
    void probeVersion();
    return Math.min(reconnectBackoffMaxMs, 1000 * 2 ** Math.max(0, attempts - 1));
  }

  function updateAvailable(): void {
    setBanner({ kind: "update" });
  }

  // Probe for a new build whenever the tab returns to the foreground. An installed
  // iOS PWA resumes its frozen page instead of re-navigating, so a deploy is
  // otherwise invisible until a manual reload (and the WS may never have dropped).
  // Unlike a silent auto-refresh this only raises the (non-intrusive) update
  // banner, so it never yanks the page out from under someone mid-read/mid-listen.
  // Returns a cleanup fn for the effect.
  function watchForegroundVersion(): () => void {
    const onVisible = (): void => {
      if (globalThis.document.visibilityState === "visible") {
        void probeVersion();
      }
    };
    globalThis.document.addEventListener("visibilitychange", onVisible);
    return () => globalThis.document.removeEventListener("visibilitychange", onVisible);
  }

  function subscribe(listener: () => void): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  function useConnectionBanner(): Banner | undefined {
    return useSyncExternalStore(
      subscribe,
      () => banner,
      () => banner,
    );
  }

  function version(): string | undefined {
    return knownVersion;
  }

  // Capture knownVersion NOW, at store creation = page load. The bundle we're
  // running was just served by the current server, so /version at this instant
  // matches it. Capturing here (instead of waiting for the first WS connect, which
  // can land AFTER a redeploy — the tab then records the NEW build id while running
  // the OLD bundle and never detects a change → "reconnected but didn't reload")
  // closes that race: any later /version change is a genuine redeploy → reload.
  void probeVersion();

  return {
    connectionReady,
    connectionLost,
    updateAvailable,
    downloadUpdate,
    reloadIntoUpdate,
    watchForegroundVersion,
    useConnectionBanner,
    version,
  };
}

// Seconds the update bar counts down before reloading on its own.
const DEFAULT_UPDATE_COUNTDOWN_SECS = 3;

export interface AutoUpdateOptions {
  /** Seconds the countdown runs before the update is applied. Default 3. */
  readonly countdownSecs?: number;
  /** Whether the surface is idle enough to be replaced right now. Default: always idle. */
  readonly canApplyUpdate?: (() => boolean) | undefined;
  /** Foreground dwell required before a reload. Default 0 — no dwell. */
  readonly minVisibleMs?: number;
  /** How long to wait after a download that did not finish. Default 60 s. */
  readonly retryMs?: number;
  /** Set false on a surface that does not present the update state at all. */
  readonly enabled?: boolean;
}

/** A pending update, reduced to what a surface has to render. */
export interface AutoUpdateState {
  /** A deployed build is waiting for this page. */
  readonly pending: boolean;
  /** Where the update is: fetching its bits, holding them, swapping into them,
   *  or stalled with them incomplete. */
  readonly phase: UpdatePhase;
  /** 0–1 once the boot-asset count is known; `undefined` while it is not, and
   *  on a page no service worker controls — there is nothing to pre-fetch
   *  there, so a fill would be a lie. */
  readonly progress: number | undefined;
  /** A batch landed mid-download. The only case where animating a fill tells
   *  the truth: a build that was already cached jumps to 1 with nothing to
   *  narrate, and a swept progress bar there would be theatre. */
  readonly streamed: boolean;
  /** The user pressed while the bits were still coming, and the page reloads as
   *  soon as they are here. */
  readonly requested: boolean;
  /** Seconds left in the visible countdown. It only runs once `ready`. */
  readonly secs: number;
  /** The countdown is parked because the user is busy or just resumed. */
  readonly held: boolean;
  /** Press the control: take the update now, or take back the request. */
  requestUpdate(): void;
}

/** What the download has produced so far. */
interface DownloadState {
  /** Set once the download settles. */
  readonly result: UpdateDownload | undefined;
  readonly done: number;
  readonly total: number;
  readonly streamed: boolean;
}

const IDLE_DOWNLOAD: DownloadState = { result: undefined, done: 0, total: 0, streamed: false };

// Apply a deployed build on the page's own initiative. Every surface runs the same
// policy (docs/offline-first-sync.md §Update policy): fetch the new build the moment
// it is detected, then count down while the user is idle, rewind whenever they are
// not, and reload into the cached bits. A press only brings that reload forward —
// the page still updates on its own for anyone who never presses.
export function useAutoUpdate(store: ConnectionStore, options: AutoUpdateOptions = {}): AutoUpdateState {
  const {
    countdownSecs = DEFAULT_UPDATE_COUNTDOWN_SECS,
    canApplyUpdate,
    minVisibleMs = 0,
    retryMs = UPDATE_RETRY_MS,
    enabled = true,
  } = options;
  const banner = store.useConnectionBanner();
  const pending = enabled && banner?.kind === "update";
  const [download, setDownload] = useState<DownloadState>(IDLE_DOWNLOAD);
  // Bumped to start the download over, by the retry timer or by a press.
  const [attempt, setAttempt] = useState(0);
  const [requested, setRequested] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [countdown, setCountdown] = useState<UpdateCountdown>({ secs: countdownSecs, held: false });
  const [visibleSince, setVisibleSince] = useState(() => Date.now());
  // Re-arms the one-second check even when the countdown itself did not move. A held
  // countdown rewinds to the same value, so without this the effect's dependencies
  // never change, the next timer is never scheduled, and the update stalls for the
  // rest of the page's life.
  const [recheck, setRecheck] = useState(0);
  const downloaded = download.result === "ready" || download.result === "unsupported";

  // A resumed page earns its dwell again. iOS restores a frozen PWA instead of
  // re-navigating, and its timers were paused the whole time it was away.
  useEffect(() => {
    if (minVisibleMs <= 0) return undefined;
    const onVisibility = (): void => {
      if (globalThis.document.visibilityState === "visible") setVisibleSince(Date.now());
    };
    globalThis.document.addEventListener("visibilitychange", onVisibility);
    return () => globalThis.document.removeEventListener("visibilitychange", onVisibility);
  }, [minVisibleMs]);

  // Fetch the deployed build at once. This is deliberately ungated: it costs the
  // user nothing they can feel, and it is what turns the control into a promise
  // the page can keep — press it and the swap is local, instant and offline-safe.
  useEffect(() => {
    if (!pending) {
      setDownload(IDLE_DOWNLOAD);
      setRequested(false);
      setReloading(false);
      return undefined;
    }
    let alive = true;
    setDownload(IDLE_DOWNLOAD);
    void store.downloadUpdate((done, total) => {
      if (!alive) return;
      setDownload((current) =>
        current.result === undefined
          ? {
            result: undefined,
            done,
            total,
            streamed: current.streamed || (done > 0 && done < total),
          }
          : current
      );
    }).then((result) => {
      if (alive) setDownload((current) => ({ ...current, result }));
    });
    return () => {
      alive = false;
    };
  }, [pending, attempt, store]);

  // An unfinished download keeps this build running and tries again later. The
  // control offers that retry immediately; this is for the page nobody is
  // watching.
  useEffect(() => {
    if (download.result !== "failed") return undefined;
    const retry = setTimeout(() => setAttempt((value) => value + 1), retryMs);
    return () => clearTimeout(retry);
  }, [download.result, retryMs]);

  // The automatic countdown. It starts only once the bits are here, so the
  // seconds it shows are the whole remaining wait and never strand the user on
  // "0s" while a download catches up.
  useEffect(() => {
    if (!pending || !downloaded || reloading) {
      setCountdown({ secs: countdownSecs, held: false });
      return undefined;
    }
    // 3 means three real seconds: show 3, 2, 1, then swap as the counter reaches
    // zero. Waiting for -1 made the nominal three-second countdown last four.
    if (countdown.secs <= 0) return undefined;
    const t = setTimeout(() => {
      const allowed = updateAllowed({
        idle: canApplyUpdate === undefined || canApplyUpdate(),
        visible: globalThis.document?.visibilityState !== "hidden",
        visibleForMs: Date.now() - visibleSince,
      }, minVisibleMs);
      setCountdown((current) => tickUpdateCountdown(current, allowed, countdownSecs));
      setRecheck((value) => value + 1);
    }, 1000);
    return () => clearTimeout(t);
  }, [
    pending,
    downloaded,
    reloading,
    countdown.secs,
    recheck,
    countdownSecs,
    canApplyUpdate,
    minVisibleMs,
    visibleSince,
  ]);

  // The one place a running build is replaced, on either road (update-policy
  // `updateReloadsNow`): the countdown ran out, or the user asked.
  useEffect(() => {
    const result = download.result;
    if (!pending || reloading || result === undefined || result === "failed") return undefined;
    if (!updateReloadsNow({ downloaded: true, requested, countedDown: countdown.secs <= 0 })) {
      return undefined;
    }
    setReloading(true);
    void store.reloadIntoUpdate(result);
    return undefined;
  }, [pending, reloading, download.result, requested, countdown.secs, store]);

  const requestUpdate = useCallback((): void => {
    if (!pending || reloading) return;
    if (download.result === "failed") {
      // One press is both "try again" and "and then take it": nobody who asks
      // for the update wants to ask a second time once it arrives.
      setRequested(true);
      setAttempt((value) => value + 1);
      return;
    }
    setRequested((was) => !was);
  }, [pending, reloading, download.result]);

  const fraction = download.total > 0 ? Math.min(1, download.done / download.total) : undefined;
  return {
    pending,
    phase: reloading
      ? "reloading"
      : download.result === "failed"
      ? "failed"
      : downloaded
      ? "ready"
      : "downloading",
    // `unsupported` never downloaded anything; it is ready to press, with
    // nothing honest to show as progress.
    progress: download.result === "ready" ? 1 : download.result === "unsupported" ? undefined : fraction,
    streamed: download.streamed,
    requested,
    secs: countdown.secs,
    held: countdown.held,
    requestUpdate,
  };
}

// MUI palette per banner kind: an outage is a calm WARNING (yellow), not an
// alarm — the app stays fully usable offline and retries are unbounded, so red
// would overstate it. Green recovery flash; blue (info) update.
function bannerPalette(kind: BannerKind): "warning" | "success" | "info" {
  if (kind === "down") {
    return "warning";
  }
  if (kind === "reconnected") {
    return "success";
  }
  return "info";
}

// Liveview's exact English labels. The update line narrates its phase: the
// download's progress, then the live 3→0 countdown, and in every state it names
// the press that brings the reload forward.
function bannerLabel(kind: BannerKind, update: AutoUpdateState): string {
  if (kind === "down") {
    return "Connection lost — reconnecting…";
  }
  if (kind === "reconnected") {
    return "Reconnected";
  }
  if (update.phase === "reloading") {
    return "Reloading into the new version…";
  }
  if (update.phase === "failed") {
    return update.requested
      ? "New version · download paused, retrying"
      : "New version · download paused, click to retry";
  }
  if (update.phase === "downloading") {
    const percent = updatePercentLabel(update.progress, false);
    const suffix = percent === undefined ? "downloading…" : percent;
    return update.requested
      ? `Reloading when ready · ${suffix}`
      : `New version · ${suffix}`;
  }
  if (update.held) {
    return "New version ready · click to reload";
  }
  return `New version · reloading in ${String(Math.max(0, update.secs))}s`;
}

export interface ConnectionBannerProps {
  readonly store: ConnectionStore;
  /** Seconds the update bar counts down before reloading. Default 3. */
  readonly countdownSecs?: number;
  /** Which banner kinds this surface renders. Default: all three. An app that
   *  presents connectivity elsewhere (a status pill) keeps only `update`. */
  readonly kinds?: readonly BannerKind[];
  /** Whether the pending update may reload the page right now. While it
   *  answers false the countdown holds at its start and the label says so; the
   *  reload happens only after the gate has stayed open for the whole count. */
  readonly canApplyUpdate?: () => boolean;
}

// Full-width overlay bar tracking the app's socket + build version. All three
// states are the SAME bar — `position: fixed` keeps it on top of everything and
// out of the layout flow, so it never pushes content down or disturbs whatever
// the user is doing:
//   - red "down"          — a sustained reconnect failure (spinner);
//   - green "reconnected"  — recovery, auto-dismissed (check);
//   - blue "update"        — a redeploy was detected; the bar fills with the
//                            download, counts 3→0 and reloads into the new
//                            build on its own.
// The two informational states never eat taps meant for the chrome underneath.
// The update state is the exception: the whole bar is the control, which is why
// it is the one that may be pressed — a full-width target needs no aim, and
// pressing it only brings forward a reload that was coming anyway.
export function ConnectionBanner(props: ConnectionBannerProps): ReactNode {
  const { store, countdownSecs = DEFAULT_UPDATE_COUNTDOWN_SECS, kinds, canApplyUpdate } = props;
  const rawBanner = store.useConnectionBanner();
  const banner = rawBanner !== undefined && kinds !== undefined && !kinds.includes(rawBanner.kind)
    ? undefined
    : rawBanner;
  // One shared policy drives every surface: the countdown runs while the user is
  // idle and rewinds whenever they are not, then the page reloads itself.
  const update = useAutoUpdate(store, {
    countdownSecs,
    canApplyUpdate,
    enabled: kinds === undefined || kinds.includes("update"),
  });

  if (!banner) {
    return null;
  }

  const palette = bannerPalette(banner.kind);
  const label = bannerLabel(banner.kind, update);
  const isUpdate = banner.kind === "update";
  const busy = update.phase === "reloading" ||
    (update.phase === "downloading" && update.progress === undefined);
  const barSx: SxProps<Theme> = (theme) => ({
    position: "fixed",
    top: 0,
    left: 0,
    right: 0,
    width: "100%",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    gap: 1,
    px: 2,
    py: 0.75,
    // Owns the notch when shown (it's the topmost element).
    pt: "calc(var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px)) + 6px)",
    backgroundColor: theme.palette[palette].main,
    color: theme.palette[palette].contrastText,
    fontSize: "0.8125rem",
    fontWeight: 500,
    pointerEvents: isUpdate ? "auto" : "none",
    zIndex: theme.zIndex.tooltip + 1,
    ...(isUpdate
      ? updateFillSx(
        theme.palette.info.main,
        theme.palette.info.dark,
        updateFillShare(update.phase, update.progress),
        update.streamed,
      )
      : {}),
  });
  const content = (
    <>
      {banner.kind === "down" && <CircularProgress size={14} color="inherit" thickness={5} />}
      {banner.kind === "reconnected" && <CheckIcon sx={{ fontSize: "1.125rem" }} />}
      {isUpdate && busy && <CircularProgress size={14} color="inherit" thickness={5} />}
      <span>{label}</span>
    </>
  );

  if (isUpdate) {
    return (
      <ButtonBase
        aria-live="polite"
        aria-label={label}
        // No ripple: the bar is its own progress fill, and a ripple would lay a
        // second animated layer over it. The press already answers in the
        // label, which is the feedback that means anything here.
        disableRipple
        disabled={update.phase === "reloading"}
        onClick={update.requestUpdate}
        sx={barSx}
      >
        {content}
      </ButtonBase>
    );
  }
  return (
    <Box role="status" aria-live="polite" sx={barSx}>
      {content}
    </Box>
  );
}
