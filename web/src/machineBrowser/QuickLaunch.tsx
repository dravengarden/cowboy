/** The quick-launch row of the Machine browser panel and its pending link:
 * a tap navigates the connected viewer at once, or opens (attaches to, or
 * takes over) the browser and navigates exactly once when this panel's
 * viewer has connected to it. A pending link is bound to the browser
 * identity it was tapped for: an open answering another identity, a failed
 * attachment or a cancelled takeover drops it, and a re-bound panel drops
 * it too, so it never reaches another profile or another viewer's page. */
import { Button, Stack } from "@mui/material";
import { useEffect, useState } from "react";
import type { BrowserTarget } from "./api";
import type { ConfirmKind } from "./confirmation";
import type { BrowserCommands } from "./MachineBrowserActions";
import { shownTarget } from "./browserModel";
import {
  type PendingQuickLink,
  type QuickLink,
  panelIdentity,
  quickDisabled,
  quickLaunchPlan,
  quickStep,
  viewerOf,
} from "./quickLinks";
import type { MachineBrowserController } from "./useMachineBrowser";

export interface QuickLaunch {
  launch: (url: string) => void;
  /** The panel's open (the chooser's included): a link waiting for the
   * Machine choice opens there, with the identity it was tapped for. */
  open: (machine?: string, expected?: string) => void;
  disabled: boolean;
}

export function useQuickLaunch(
  browser: MachineBrowserController,
  commands: BrowserCommands,
  confirm: { ask: (kind: ConfirmKind) => void; showing: ConfirmKind | undefined },
  context: {
    /** This panel's connected viewer (`controlsFor`), if any: its target
     * and attachment, which every navigation here is fenced to. */
    connected: { target: BrowserTarget; attachmentId: string } | undefined;
    /** The sheet's binding (inventory) identity, known in the same render. */
    sheetIdentity: string | undefined;
    identityRequired: boolean;
  },
): QuickLaunch {
  const [pending, setPending] = useState<PendingQuickLink | undefined>(undefined);
  const { model } = browser;
  const viewer = model.busy ? undefined : context.connected;
  const connected = viewer?.target;
  const attachmentId = viewer?.attachmentId;
  const step = pending
    ? quickStep(pending, {
        identity: panelIdentity(model),
        sheetIdentity: context.sheetIdentity,
        viewer: viewerOf(model),
        connected,
        confirming: confirm.showing,
        takeoverViewer: model.viewer?.takeover === true,
        failures: model.failures ?? 0,
      })
    : undefined;
  useEffect(() => {
    if (!pending || !step) return;
    if (step.action === "wait") {
      if (step.pending !== pending) setPending(step.pending);
      return;
    }
    setPending(undefined);
    if (step.action === "navigate" && connected) {
      browser.navigate(pending.url, connected, attachmentId);
    }
  }, [attachmentId, browser, connected, pending, step]);
  const launch = (url: string): void => {
    const plan = quickLaunchPlan(model, connected, context.identityRequired);
    if (plan === "none") return;
    if (plan === "navigate" && connected) {
      browser.navigate(url, connected, attachmentId);
      return;
    }
    const identity = panelIdentity(model);
    const target = model.read?.state ? shownTarget(model) : undefined;
    setPending({
      url,
      identity,
      takeover: plan === "takeover",
      failures: model.failures ?? 0,
      sinceEpoch: model.epoch ?? 0,
      // Reuse binds to the attachment already connecting; attaching and
      // taking over stay on the shown browser's Machine.
      viewer: plan === "reuse" ? viewerOf(model) : undefined,
      machine: plan === "attach" || plan === "takeover" ? target?.machine_id : undefined,
    });
    // "choose": the first open waits for the Machine picked in the chooser.
    if (plan === "open") commands.open(undefined, identity);
    else if (plan === "attach") commands.attach(false);
    else if (plan === "takeover") confirm.ask("takeover");
  };
  const open = (machine?: string, expected?: string): void => {
    if (pending && !pending.viewer && machine !== undefined) {
      setPending({ ...pending, machine });
      commands.open(machine, pending.identity);
      return;
    }
    commands.open(machine, expected);
  };
  return { launch, open, disabled: quickDisabled(model, context.identityRequired) };
}

export function QuickLinkList({
  links,
  disabled,
  onLaunch,
}: {
  links: readonly QuickLink[];
  disabled: boolean;
  onLaunch: (url: string) => void;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      spacing={1}
      useFlexGap
      flexWrap="wrap"
      justifyContent="center"
      aria-label="Quick links"
      role="group"
      data-machine-browser-quick-links
      sx={{ maxWidth: 480 }}
    >
      {links.map((link) => (
        <Button
          key={link.key}
          size="small"
          variant={link.emphasized ? "contained" : "outlined"}
          color={link.emphasized ? "secondary" : "inherit"}
          disabled={disabled}
          aria-label={`Open ${link.label} in this browser`}
          data-quick-link={link.key}
          onClick={() => onLaunch(link.url)}
          sx={{ minHeight: 36, borderRadius: 999 }}
        >
          {link.label}
        </Button>
      ))}
    </Stack>
  );
}
