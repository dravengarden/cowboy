/** The page area of the Machine browser panel: the attached display filling
 * the frame, or, while there is none, a browser-like empty page with one
 * primary action. Errors and notices float over the frame as dismissible
 * banners so the frame (and the framebuffer size it requests) never moves. */
import { GroupsRounded, PublicOutlined } from "@mui/icons-material";
import { Alert, Box, Button, Stack, Typography } from "@mui/material";
import { type RefObject, Suspense, lazy } from "react";
import { type BrowserStateEvent, browserErrorText } from "./api";
import { type ViewerControls, browserStateSummary } from "./browserChrome";
import { type BrowserModel, browserRunning } from "./browserModel";
import type { ConfirmKind } from "./confirmation";
import { type BrowserCommands, BrowserPrimaryAction } from "./MachineBrowserActions";
import type { SharedBrowser } from "./shareRules";
import type { MachineBrowserController } from "./useMachineBrowser";

const MachineBrowserViewer = lazy(() => import("./MachineBrowserViewer"));

const EMPTY_TITLE: Record<string, string> = {
  hibernated: "This browser is hibernating",
  starting: "Starting the browser",
  detached: "The browser is running",
  attached: "Another device is viewing this browser",
  stopping: "The browser is stopping",
};

/** A banner already names the reason of a failure. */
function withoutReason(state: BrowserStateEvent | null): BrowserStateEvent | null {
  if (!state) return state;
  const { reason, ...rest } = state;
  void reason;
  return rest;
}

function BrowserEmptyPage({
  model,
  state,
  machineLabel,
  link,
  pinned,
  commands,
  enabling,
  onEnableBrowser,
  onConfirm,
  shared,
}: {
  model: BrowserModel;
  state: BrowserStateEvent | null;
  machineLabel: string;
  link: boolean;
  pinned: boolean;
  commands: BrowserCommands;
  enabling: boolean;
  onEnableBrowser: (() => void) | undefined;
  onConfirm: (kind: ConfirmKind) => void;
  shared: SharedBrowser | undefined;
}): React.JSX.Element {
  const admission = model.read?.admission;
  const stateTitle = !model.loaded
    ? "Reading the browser state…"
    : admission
      ? "The browser is not available"
      : (EMPTY_TITLE[state?.state ?? ""] ?? "This browser was never opened");
  const title = shared ? `Shared browser · ${shared.label}` : stateTitle;
  return (
    <Stack
      spacing={1.5}
      alignItems="center"
      justifyContent="center"
      data-machine-browser-empty
      sx={{ flex: 1, minHeight: 0, px: 3, py: 4, textAlign: "center", overflowY: "auto" }}
    >
      {shared ? (
        <GroupsRounded
          role="img"
          aria-label="Shared browser"
          sx={{ fontSize: "3rem", color: "secondary.main" }}
        />
      ) : (
        <PublicOutlined sx={{ fontSize: "3rem", color: "text.disabled" }} />
      )}
      <Typography
        variant="h6"
        component="p"
        role="status"
        data-machine-browser-title={shared ? "shared" : "own"}
        sx={{ fontWeight: 600, color: shared ? "secondary.main" : undefined }}
      >
        {title}
      </Typography>
      {shared && (
        <Typography variant="body2" sx={{ fontWeight: 600 }}>
          {stateTitle}
        </Typography>
      )}
      <Typography variant="body2" color="text.secondary" sx={{ maxWidth: 420 }}>
        {admission
          ? browserErrorText(admission)
          : model.loaded
            ? `${browserStateSummary(model.error ? withoutReason(state) : state)} · Machine: ${machineLabel}`
            : `Machine: ${machineLabel}`}
      </Typography>
      {admission === "browser_not_enabled" && onEnableBrowser && (
        <Button variant="contained" disabled={enabling} onClick={onEnableBrowser}>
          Enable browser for this account
        </Button>
      )}
      <BrowserPrimaryAction
        model={model}
        state={state}
        pinned={pinned}
        commands={commands}
        onConfirm={onConfirm}
      />
      {link && (
        <Typography variant="caption" color="text.secondary" sx={{ maxWidth: 420 }}>
          The link opens in this account’s browser once it runs. Anything the page shows you (such
          as a code) stays yours to copy back.
        </Typography>
      )}
    </Stack>
  );
}

function BrowserBanners({
  model,
  retry,
  commands,
}: {
  model: BrowserModel;
  retry: boolean;
  commands: BrowserCommands;
}): React.JSX.Element | null {
  if (!model.error && !model.notice) return null;
  return (
    <Stack
      spacing={1}
      sx={{
        position: "absolute",
        top: 8,
        left: 8,
        right: 8,
        zIndex: 2,
        mx: "auto",
        maxWidth: 640,
        pointerEvents: "none",
        "& > *": { pointerEvents: "auto" },
      }}
    >
      {model.error && (
        <Alert severity="warning" onClose={commands.dismiss} sx={{ boxShadow: 6 }}>
          {model.error}
          {retry && (
            <Box sx={{ mt: 0.5, ml: -1 }}>
              <Button color="inherit" size="small" onClick={() => commands.open()}>
                Retry
              </Button>
            </Box>
          )}
        </Alert>
      )}
      {model.notice && (
        <Alert severity="info" onClose={commands.dismiss} sx={{ boxShadow: 6 }}>
          {model.notice}
        </Alert>
      )}
    </Stack>
  );
}

export interface BrowserFrameProps {
  areaRef: RefObject<HTMLDivElement | null>;
  browser: MachineBrowserController;
  account: { id: string; label: string };
  machineLabel: string;
  /** Touch input (`useTouchInput`), not the panel layout. */
  touch: boolean;
  link: boolean;
  pinned: boolean;
  retry: boolean;
  commands: BrowserCommands;
  enabling: boolean;
  onEnableBrowser: (() => void) | undefined;
  onConfirm: (kind: ConfirmKind) => void;
  onControls: (controls: ViewerControls | undefined) => void;
  onKeyboard: (open: boolean) => void;
  /** The shared browser this card uses, if any. */
  shared?: SharedBrowser | undefined;
}

/** The frame is the measured area: opens request the screen mode for it and
 * the mounted viewer re-requests only when that mode changes. */
export function BrowserFrame(props: BrowserFrameProps): React.JSX.Element {
  const { areaRef, browser, account, touch, commands } = props;
  const { model, dispatch } = browser;
  const state = model.read?.state ?? null;
  return (
    <Box
      ref={areaRef}
      data-machine-browser-frame
      sx={{
        flex: 1,
        minHeight: 0,
        position: "relative",
        display: "flex",
        flexDirection: "column",
        bgcolor: "background.default",
      }}
    >
      {model.viewer && browserRunning(state?.state) ? (
        <Suspense
          fallback={
            <Typography role="status" variant="body2" sx={{ p: 2, textAlign: "center" }}>
              Loading the viewer…
            </Typography>
          }
        >
          <MachineBrowserViewer
            key={model.viewer.generation}
            account={account.id}
            target={model.viewer.target}
            takeover={model.viewer.takeover}
            touch={touch}
            onState={(next) => dispatch({ type: "stream_state", state: next })}
            onEnded={(message) => {
              dispatch({ type: "viewer_ended", message });
              browser.refresh();
            }}
            onRemoteClipboard={(text) => dispatch({ type: "remote_clipboard", text })}
            onNotice={commands.notice}
            onControls={props.onControls}
            onKeyboard={props.onKeyboard}
          />
        </Suspense>
      ) : (
        <BrowserEmptyPage
          model={model}
          state={state}
          machineLabel={props.machineLabel}
          link={props.link}
          pinned={props.pinned}
          commands={commands}
          enabling={props.enabling}
          onEnableBrowser={props.onEnableBrowser}
          onConfirm={props.onConfirm}
          shared={props.shared}
        />
      )}
      <BrowserBanners model={model} retry={props.retry} commands={commands} />
    </Box>
  );
}
