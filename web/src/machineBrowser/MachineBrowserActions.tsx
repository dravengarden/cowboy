/** Lifecycle commands of the Machine browser panel: the primary action the
 * empty page offers for the current state, and the confirmations that
 * capture the browser they act on. Pure presentation over `BrowserModel`. */
import { Button, CircularProgress, Stack, Typography } from "@mui/material";
import { type ReactNode, useEffect, useState } from "react";
import { ConfirmSheet } from "../Sheet";
import { type BrowserStateEvent, type BrowserTarget, browserStateLabel } from "./api";
import type { BrowserModel } from "./browserModel";
import {
  type ConfirmKind,
  type Confirmation,
  type ResetTarget,
  confirmationCurrent,
  confirmationFor,
} from "./confirmation";
import { MachineBrowserChooser } from "./MachineBrowserChooser";
import { needsMachineChoice } from "./entryRules";
import { type SharedBrowser, sharedHibernateNote, sharedResetNote } from "./shareRules";

/** Commands that follow a confirmation take the target it captured. */
export interface BrowserCommands {
  /** `machine` pins the open to the Machine the owner chose. */
  open(machine?: string): void;
  attach(takeover: boolean, target?: BrowserTarget): void;
  detach(): void;
  hibernate(target?: BrowserTarget): void;
  snapshot(): void;
  reset(target?: ResetTarget): void;
  restart(target?: BrowserTarget): void;
  refresh(): void;
  dismiss(): void;
  notice(message: string): void;
}

const CONFIRM_TEXT: Record<
  ConfirmKind,
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
    body: "The browser hibernates with a snapshot and reopens at this screen’s scale. Page state since the last snapshot may be lost.",
    action: "Restart",
    destructive: false,
  },
};

/** A confirmation's text; a shared browser's reset and hibernation say so
 * in their title, since they act on every member's browser. */
export function confirmText(
  kind: ConfirmKind,
  shared: SharedBrowser | undefined,
): { title: string; body: string; action: string; destructive: boolean } {
  const text = CONFIRM_TEXT[kind];
  if (!shared) return text;
  if (kind === "reset") {
    return {
      ...text,
      title: `Reset the shared browser profile (${shared.label})?`,
      body: "Cookies, logins and site data of this shared browser are deleted on its Machine. No account's Cowboy credentials are affected.",
      action: "Reset shared profile",
    };
  }
  if (kind === "hibernate") {
    return { ...text, title: `Hibernate the shared browser (${shared.label})?` };
  }
  return text;
}

/** The one thing to do next, shown on the empty page inside the frame. */
export function BrowserPrimaryAction({
  model,
  state,
  pinned = false,
  commands,
  onConfirm,
}: {
  model: BrowserModel;
  state: BrowserStateEvent | null;
  /** The panel's opens are pinned to one Machine: no choice is offered. */
  pinned?: boolean;
  commands: BrowserCommands;
  onConfirm: (kind: ConfirmKind) => void;
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
  if ((!state || state.state === "hibernated") && !pinned && needsMachineChoice(model.read)) {
    return (
      <MachineBrowserChooser
        read={model.read}
        busy={Boolean(model.busy)}
        onOpen={(machine) => commands.open(machine)}
      />
    );
  }
  const primary = (label: string, onClick: () => void): React.JSX.Element => (
    <Button
      variant="contained"
      size="large"
      disabled={Boolean(model.busy)}
      onClick={onClick}
      sx={{ minHeight: 46, minWidth: 180, borderRadius: 999 }}
    >
      {label}
    </Button>
  );
  if (!state || state.state === "hibernated") return primary("Open browser", () => commands.open());
  if (state.state === "detached") return primary("Open browser", () => commands.attach(false));
  if (state.state === "attached") return primary("Take over…", () => onConfirm("takeover"));
  return <Typography variant="body2">The browser is stopping…</Typography>;
}

/** Send exactly the confirmed command to the browser it captured. */
function runConfirmed(confirm: Confirmation, commands: BrowserCommands): void {
  switch (confirm.kind) {
    case "takeover":
      commands.attach(true, confirm.target);
      break;
    case "hibernate":
      commands.hibernate(confirm.target);
      break;
    case "reset":
      commands.reset(confirm.target);
      break;
    case "restart":
      commands.restart(confirm.target);
      break;
    default:
      break;
  }
}

/** Confirmations for the panel: `ask` captures the browser shown now; the
 * dialog closes without acting once the panel shows another one. */
export function useBrowserConfirm(
  model: BrowserModel,
  state: BrowserStateEvent | null,
  commands: BrowserCommands,
  shared?: SharedBrowser,
  identityRequired = false,
): { ask: (kind: ConfirmKind) => void; dialog: ReactNode } {
  const [confirm, setConfirm] = useState<Confirmation | undefined>(undefined);
  const current = confirm !== undefined && confirmationCurrent(confirm, model, identityRequired);
  useEffect(() => {
    if (confirm && !current) setConfirm(undefined);
  }, [confirm, current]);
  const ask = (kind: ConfirmKind): void =>
    setConfirm(confirmationFor(kind, model, identityRequired));
  const text = confirm && current ? confirmText(confirm.kind, shared) : undefined;
  const confirmed = (): void => {
    if (confirm && current) runConfirmed(confirm, commands);
    setConfirm(undefined);
  };
  const dialog = text ? (
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
      {confirm?.kind === "reset" && sharedResetNote(shared) && (
        <Typography variant="body2" color="warning.main" data-shared-confirm-note>
          {sharedResetNote(shared)}
        </Typography>
      )}
      {confirm?.kind === "hibernate" && sharedHibernateNote(shared) && (
        <Typography variant="body2" color="secondary.main" data-shared-confirm-note>
          {sharedHibernateNote(shared)}
        </Typography>
      )}
      {confirm?.kind === "takeover" && (
        <Typography variant="caption" color="text.secondary">
          Current state: {browserStateLabel(state?.state)}.
        </Typography>
      )}
    </ConfirmSheet>
  ) : null;
  return { ask, dialog };
}
