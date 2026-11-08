/** The command surface of the Machine browser panel: the primary action for
 * the current state, the lifecycle bar with its confirmations, and the
 * remote-clipboard row. Pure presentation over `BrowserModel`. */
import { Button, CircularProgress, Stack, Typography } from "@mui/material";
import { useState } from "react";
import { ConfirmSheet } from "../Sheet";
import { copyText } from "../clipboard";
import { type BrowserScale, type BrowserStateEvent, browserStateLabel } from "./api";
import { type BrowserModel, browserRunning, looksLikeAuthorizationCode } from "./browserModel";

export interface BrowserCommands {
  open(): void;
  attach(takeover: boolean): void;
  detach(): void;
  hibernate(): void;
  snapshot(): void;
  reset(): void;
  restart(): void;
  refresh(): void;
  dismiss(): void;
  notice(message: string): void;
}

type Confirm = "takeover" | "hibernate" | "reset" | "restart";

const CONFIRM_TEXT: Record<
  Confirm,
  { title: string; body: string; action: string; destructive: boolean }
> = {
  takeover: {
    title: "Take over this browser?",
    body: "Another viewer is attached. Taking over detaches it; the page keeps its state.",
    action: "Take over",
    destructive: false,
  },
  hibernate: {
    title: "Hibernate the browser?",
    body: "The profile is snapshotted and the process stops. Open it again later to continue where it left off.",
    action: "Hibernate",
    destructive: false,
  },
  reset: {
    title: "Reset the browser profile?",
    body: "Cookies, logins and site data of this account’s browser are deleted on its Machine. The account’s Cowboy credentials are not affected.",
    action: "Reset profile",
    destructive: true,
  },
  restart: {
    title: "Restart for this screen?",
    body: "The browser hibernates with a snapshot and reopens at this screen’s scale. Page state since the last snapshot may be lost; a sign-in browser returns to its sign-in page.",
    action: "Restart",
    destructive: false,
  },
};

/** The one thing to do next, shown large where the display will be. */
export function BrowserPrimaryAction({
  model,
  state,
  commands,
  onConfirm,
}: {
  model: BrowserModel;
  state: BrowserStateEvent | null;
  commands: BrowserCommands;
  onConfirm: (kind: Confirm) => void;
}): React.JSX.Element | null {
  if (model.read?.admission || !model.loaded) return null;
  if (model.busy === "open" || state?.state === "starting") {
    return (
      <Stack direction="row" spacing={1} alignItems="center" role="status">
        <CircularProgress size={18} aria-label="Opening the browser" />
        <Typography variant="body2">
          Opening the browser… a cold start can take a minute.
        </Typography>
      </Stack>
    );
  }
  if (!state || state.state === "hibernated") {
    return (
      <Button variant="contained" disabled={Boolean(model.busy)} onClick={commands.open}>
        Open browser
      </Button>
    );
  }
  if (state.state === "detached") {
    return (
      <Button
        variant="contained"
        disabled={Boolean(model.busy)}
        onClick={() => commands.attach(false)}
      >
        Attach
      </Button>
    );
  }
  if (state.state === "attached") {
    return (
      <Button
        variant="contained"
        disabled={Boolean(model.busy)}
        onClick={() => onConfirm("takeover")}
      >
        Take over…
      </Button>
    );
  }
  return <Typography variant="body2">The browser is stopping…</Typography>;
}

export function BrowserActionBar({
  model,
  state,
  desktop,
  signIn,
  preferredScale,
  commands,
  onUseCode,
  onClose,
}: {
  model: BrowserModel;
  state: BrowserStateEvent | null;
  desktop: boolean;
  signIn: boolean;
  preferredScale: BrowserScale;
  commands: BrowserCommands;
  onUseCode?: ((code: string) => void) | undefined;
  onClose: () => void;
}): React.JSX.Element {
  const [confirm, setConfirm] = useState<Confirm | undefined>(undefined);
  const busy = Boolean(model.busy);
  const running = browserRunning(state?.state);
  const admitted = model.loaded && !model.read?.admission;
  const resettable =
    admitted &&
    !running &&
    state?.state !== "starting" &&
    (state !== null || (model.read?.placements.length ?? 0) > 0);
  const scaleMismatch = running && state?.scale !== undefined && state.scale !== preferredScale;
  const remote = model.remoteClipboard;
  const confirmed = (): void => {
    switch (confirm) {
      case "takeover":
        commands.attach(true);
        break;
      case "hibernate":
        commands.hibernate();
        break;
      case "reset":
        commands.reset();
        break;
      case "restart":
        commands.restart();
        break;
      default:
        break;
    }
    setConfirm(undefined);
  };
  const text = confirm ? CONFIRM_TEXT[confirm] : undefined;
  const minHeight = desktop ? 32 : 44;
  return (
    <Stack spacing={1} data-machine-browser-actions sx={{ "& .MuiButton-root": { minHeight } }}>
      {remote !== undefined && (
        <Stack direction="row" spacing={1} alignItems="center" flexWrap="wrap" useFlexGap>
          <Typography variant="caption" color="text.secondary">
            Remote clipboard: {[...remote].length} characters
          </Typography>
          <Button
            size="small"
            onClick={() => {
              copyText(remote).then((copied) =>
                commands.notice(
                  copied
                    ? "Remote clipboard copied to this device."
                    : "Could not copy on this device.",
                ),
              );
            }}
          >
            Copy to this device
          </Button>
          {signIn && onUseCode && looksLikeAuthorizationCode(remote) && (
            <Button size="small" variant="contained" onClick={() => onUseCode(remote.trim())}>
              Use as authorization code
            </Button>
          )}
        </Stack>
      )}
      <Stack direction="row" spacing={1} flexWrap="wrap" useFlexGap alignItems="center">
        {!model.viewer && (
          <BrowserPrimaryAction
            model={model}
            state={state}
            commands={commands}
            onConfirm={setConfirm}
          />
        )}
        {model.viewer && (
          <Button disabled={busy} onClick={commands.detach}>
            Stop viewing
          </Button>
        )}
        {running && (
          <Button disabled={busy} onClick={() => setConfirm("hibernate")}>
            Hibernate…
          </Button>
        )}
        {running && (
          <Button disabled={busy} onClick={commands.snapshot}>
            Snapshot
          </Button>
        )}
        {scaleMismatch && (
          <Button disabled={busy} onClick={() => setConfirm("restart")}>
            Restart for this screen…
          </Button>
        )}
        {resettable && (
          <Button disabled={busy} color="error" onClick={() => setConfirm("reset")}>
            Reset profile…
          </Button>
        )}
        <Button disabled={busy} onClick={commands.refresh}>
          Refresh
        </Button>
        {!desktop && <Button onClick={onClose}>Close</Button>}
      </Stack>
      {text && (
        <ConfirmSheet
          open
          onClose={() => setConfirm(undefined)}
          title={text.title}
          actions={
            <Stack direction="row" spacing={1}>
              <Button onClick={() => setConfirm(undefined)}>Cancel</Button>
              <Button
                variant="contained"
                color={text.destructive ? "error" : "primary"}
                onClick={confirmed}
              >
                {text.action}
              </Button>
            </Stack>
          }
        >
          <Typography variant="body2">{text.body}</Typography>
          {confirm === "takeover" && (
            <Typography variant="caption" color="text.secondary">
              Current state: {browserStateLabel(state?.state)}.
            </Typography>
          )}
        </ConfirmSheet>
      )}
    </Stack>
  );
}
