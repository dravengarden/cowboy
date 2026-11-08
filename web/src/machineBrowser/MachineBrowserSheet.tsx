/** The panel an account card opens for its Machine browser: state, lifecycle
 * actions and, while running, the attached display. Desktop is a wide
 * workbench dialog; Mobile is a cover sheet with the controls at the bottom.
 * See docs/machine-browser.md, "Viewer (core web)". */
import { ArrowBackIosNewRounded, CloseRounded } from "@mui/icons-material";
import { Alert, Box, Button, IconButton, Stack, Typography } from "@mui/material";
import { type RefObject, Suspense, lazy, useEffect, useRef } from "react";
import { Sheet } from "../Sheet";
import { useStoreSelector } from "../store";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import {
  type BrowserScale,
  type BrowserStateEvent,
  type BrowserViewport,
  browserErrorText,
  browserReasonText,
  browserStateLabel,
  browserViewport,
} from "./api";
import { type BrowserModel, autoEntry, browserRunning } from "./browserModel";
import { BrowserActionBar, type BrowserCommands } from "./MachineBrowserActions";
import { type MachineBrowserController, useMachineBrowser } from "./useMachineBrowser";

const MachineBrowserViewer = lazy(() => import("./MachineBrowserViewer"));

export interface MachineBrowserSheetProps {
  open: boolean;
  onClose: () => void;
  account: { id: string; label: string; browserEnabled: boolean };
  owner: string;
  dataset: string;
  /** Sign-in mode: opens bind to this native login and the remote clipboard
   * may fill the authorization code through `onUseCode`. */
  signInRequest?: string | undefined;
  /** Open or attach as soon as the state is known. */
  autoOpen?: boolean | undefined;
  onEnableBrowser?: (() => void) | undefined;
  /** The enablement write is pending elsewhere. */
  enabling?: boolean | undefined;
  onUseCode?: ((code: string) => void) | undefined;
}

/** State line for headers and the idle panel: label, reason and grace deadline. */
export function browserStateSummary(state: BrowserStateEvent | null, now = Date.now()): string {
  const parts = [browserStateLabel(state?.state)];
  const reason = browserReasonText(state?.reason);
  if (reason) parts.push(reason);
  if (state?.state === "detached" && state.deadline_ms !== undefined) {
    const seconds = Math.max(0, Math.round((state.deadline_ms - now) / 1000));
    parts.push(`Hibernates in ${seconds} s unless attached`);
  }
  if (state?.unsaved_state_lost) parts.push("The last stop lost unsaved page state");
  return parts.join(" · ");
}

function BrowserNotices({
  model,
  enabling,
  onEnableBrowser,
  onDismiss,
}: {
  model: BrowserModel;
  enabling: boolean;
  onEnableBrowser: (() => void) | undefined;
  onDismiss: () => void;
}): React.JSX.Element | null {
  const admission = model.read?.admission;
  if (!admission && !model.error && !model.notice) return null;
  return (
    <Stack spacing={1}>
      {admission && (
        <Alert
          severity="info"
          action={
            admission === "browser_not_enabled" && onEnableBrowser ? (
              <Button size="small" disabled={enabling} onClick={onEnableBrowser}>
                Enable browser for this account
              </Button>
            ) : undefined
          }
        >
          {browserErrorText(admission)}
        </Alert>
      )}
      {model.error && (
        <Alert severity="warning" onClose={onDismiss}>
          {model.error}
        </Alert>
      )}
      {model.notice && (
        <Alert severity="info" onClose={onDismiss}>
          {model.notice}
        </Alert>
      )}
    </Stack>
  );
}

function BrowserIdle({
  model,
  state,
  machineLabel,
  signIn,
}: {
  model: BrowserModel;
  state: BrowserStateEvent | null;
  machineLabel: string;
  signIn: boolean;
}): React.JSX.Element {
  return (
    <Stack
      spacing={1}
      alignItems="center"
      justifyContent="center"
      sx={{
        flex: 1,
        minHeight: 0,
        p: 2,
        textAlign: "center",
        borderRadius: 1,
        bgcolor: "action.hover",
      }}
    >
      <Typography variant="subtitle2" role="status">
        {model.loaded ? browserStateSummary(state) : "Reading the browser state…"}
      </Typography>
      <Typography variant="caption" color="text.secondary">
        Machine: {machineLabel}
      </Typography>
      {signIn && (
        <Typography variant="caption" color="text.secondary">
          Complete the sign-in inside this browser. When the page shows an authorization code, copy
          it there; it appears here as the remote clipboard.
        </Typography>
      )}
    </Stack>
  );
}

/** The display area: the lazily loaded viewer while attached, else the state. */
function BrowserDisplay({
  areaRef,
  browser,
  account,
  machineLabel,
  scale,
  desktop,
  signIn,
  onNotice,
}: {
  areaRef: RefObject<HTMLDivElement | null>;
  browser: MachineBrowserController;
  account: { id: string; label: string };
  machineLabel: string;
  scale: BrowserScale;
  desktop: boolean;
  signIn: boolean;
  onNotice: (message: string) => void;
}): React.JSX.Element {
  const { model, dispatch } = browser;
  const state = model.read?.state ?? null;
  return (
    <Box ref={areaRef} sx={{ flex: 1, minHeight: 240, display: "flex", flexDirection: "column" }}>
      {model.viewer && browserRunning(state?.state) ? (
        <Suspense
          fallback={
            <Typography role="status" variant="body2" sx={{ p: 2 }}>
              Loading the viewer…
            </Typography>
          }
        >
          <MachineBrowserViewer
            key={model.viewer.generation}
            account={account.id}
            accountLabel={account.label}
            machineLabel={machineLabel}
            scale={scale}
            takeover={model.viewer.takeover}
            touch={!desktop}
            onState={(next) => dispatch({ type: "stream_state", state: next })}
            onEnded={(message) => {
              dispatch({ type: "viewer_ended", message });
              browser.refresh();
            }}
            onRemoteClipboard={(text) => dispatch({ type: "remote_clipboard", text })}
            onNotice={onNotice}
          />
        </Suspense>
      ) : (
        <BrowserIdle model={model} state={state} machineLabel={machineLabel} signIn={signIn} />
      )}
    </Box>
  );
}

function MobileBrowserHeader({
  title,
  summary,
  onBack,
}: {
  title: string;
  summary: string;
  onBack: () => void;
}): React.JSX.Element {
  return (
    <Stack direction="row" alignItems="center" spacing={0.5} sx={{ pl: 0.5, pr: 2, minHeight: 48 }}>
      <IconButton
        aria-label="Back"
        onClick={onBack}
        onPointerDown={(event) => event.stopPropagation()}
        sx={{ width: 44, height: 44 }}
      >
        <ArrowBackIosNewRounded fontSize="small" />
      </IconButton>
      <Stack sx={{ flex: 1, minWidth: 0 }}>
        <Typography variant="subtitle2" noWrap>
          {title}
        </Typography>
        <Typography variant="caption" color="text.secondary" noWrap>
          {summary}
        </Typography>
      </Stack>
    </Stack>
  );
}

function DesktopBrowserHeader({
  title,
  summary,
  onClose,
}: {
  title: string;
  summary: string;
  onClose: () => void;
}): React.JSX.Element {
  return (
    <Stack direction="row" alignItems="center" spacing={1} sx={{ minWidth: 0 }}>
      <Stack sx={{ flex: 1, minWidth: 0 }}>
        <Typography variant="subtitle1" noWrap sx={{ fontWeight: 700 }}>
          {title}
        </Typography>
        <Typography variant="caption" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>
          {summary}
        </Typography>
      </Stack>
      <IconButton aria-label="Close browser" size="small" onClick={onClose}>
        <CloseRounded fontSize="small" />
      </IconButton>
    </Stack>
  );
}

/** Commands over the controller; the viewport is measured from the display
 * area at the moment of the request. */
function browserCommands(
  browser: MachineBrowserController,
  areaRef: RefObject<HTMLDivElement | null>,
  scale: BrowserScale,
  signIn: boolean,
): BrowserCommands {
  const viewportNow = (): BrowserViewport => {
    const rect = areaRef.current?.getBoundingClientRect();
    return browserViewport(
      rect?.width || globalThis.innerWidth,
      rect?.height || globalThis.innerHeight - 160,
      scale,
    );
  };
  const { dispatch } = browser;
  return {
    open: () => browser.open(viewportNow()),
    // A sign-in panel never attaches directly: the sign-in open lets the
    // Machine validate and bind this login first, then the viewer mounts.
    attach: (takeover) =>
      signIn ? browser.open(viewportNow(), takeover) : dispatch({ type: "attach", takeover }),
    detach: () => dispatch({ type: "detach" }),
    hibernate: browser.hibernate,
    snapshot: browser.snapshot,
    reset: browser.reset,
    restart: () => browser.restart(viewportNow()),
    refresh: browser.refresh,
    dismiss: () => dispatch({ type: "dismiss" }),
    notice: (message) => dispatch({ type: "notice", message }),
  };
}

export function MachineBrowserSheet(props: MachineBrowserSheetProps): React.JSX.Element {
  const {
    open,
    onClose,
    account,
    owner,
    dataset,
    signInRequest,
    autoOpen = false,
    enabling = false,
  } = props;
  const surface = useSurfaceProfile();
  const desktop = surface.kind === "desktop";
  // A phone renders Chromium at device scale 2; pointer surfaces at 1.
  const scale: BrowserScale = surface.kind === "mobile" && surface.touchCapable ? 2 : 1;
  const machines = useStoreSelector((state) => state.machines);
  const browser = useMachineBrowser({
    account: account.id,
    dataset,
    owner,
    active: open,
    browserEnabled: account.browserEnabled,
    signInRequest,
  });
  const { model } = browser;
  const areaRef = useRef<HTMLDivElement | null>(null);
  const autoOpened = useRef(false);
  const state = model.read?.state ?? null;
  const machineId = model.read?.machine_id ?? model.read?.placements[0]?.machine_id ?? null;
  const machineLabel = machineId
    ? machines.find((machine) => machine.id === machineId)?.display_name || machineId
    : "no Machine yet";
  const commands = browserCommands(browser, areaRef, scale, Boolean(signInRequest));
  const commandsRef = useRef(commands);
  commandsRef.current = commands;
  const readyToAutoOpen = autoOpen && open && model.loaded && !model.busy && !model.read?.admission;
  useEffect(() => {
    if (!readyToAutoOpen || autoOpened.current) return;
    autoOpened.current = true;
    const entry = autoEntry(Boolean(signInRequest), state?.state);
    if (entry === "attach") commandsRef.current.attach(false);
    else if (entry === "open") commandsRef.current.open();
  }, [readyToAutoOpen, signInRequest, state]);

  const signIn = Boolean(signInRequest);
  const title = signIn ? `Sign-in browser of ${account.label}` : `Browser of ${account.label}`;
  const summary = `${browserStateSummary(state)} · ${machineLabel}`;
  const body = (
    <Box
      data-machine-browser-sheet={account.id}
      sx={{
        flex: 1,
        minHeight: 0,
        display: "flex",
        flexDirection: "column",
        gap: 1,
        pt: desktop ? 0 : 1,
        px: desktop ? 0 : 2,
      }}
    >
      {desktop && <DesktopBrowserHeader title={title} summary={summary} onClose={onClose} />}
      <BrowserNotices
        model={model}
        enabling={enabling}
        onEnableBrowser={props.onEnableBrowser}
        onDismiss={commands.dismiss}
      />
      <BrowserDisplay
        areaRef={areaRef}
        browser={browser}
        account={account}
        machineLabel={machineLabel}
        scale={scale}
        desktop={desktop}
        signIn={signIn}
        onNotice={commands.notice}
      />
      <Box
        sx={{
          pb: desktop
            ? 2
            : "max(8px, calc(env(safe-area-inset-bottom, 0px) - var(--kb-inset, 0px)))",
        }}
      >
        <BrowserActionBar
          model={model}
          state={state}
          desktop={desktop}
          signIn={signIn}
          preferredScale={scale}
          commands={commands}
          onUseCode={props.onUseCode}
          onClose={onClose}
        />
      </Box>
    </Box>
  );
  if (desktop) {
    return (
      <Sheet open={open} onClose={onClose} title={title} desktopMaxWidth={1240} desktopFill>
        {body}
      </Sheet>
    );
  }
  return (
    <Sheet
      open={open}
      onClose={onClose}
      title={title}
      cover
      fill
      forceSheet
      portal
      mobileDismiss="none"
      denseHeader={<MobileBrowserHeader title={title} summary={summary} onBack={onClose} />}
    >
      {body}
    </Sheet>
  );
}
