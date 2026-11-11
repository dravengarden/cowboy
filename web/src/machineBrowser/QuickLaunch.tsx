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
import {
  type PendingQuickLink,
  type QuickLink,
  panelIdentity,
  quickLaunchPlan,
  quickStep,
} from "./quickLinks";
import type { MachineBrowserController } from "./useMachineBrowser";

export function useQuickLaunch(
  browser: MachineBrowserController,
  commands: BrowserCommands,
  confirm: { ask: (kind: ConfirmKind) => void; showing: ConfirmKind | undefined },
  /** This panel's connected viewer's target (`controlsFor`), if any. */
  connected: BrowserTarget | undefined,
): (url: string) => void {
  const [pending, setPending] = useState<PendingQuickLink | undefined>(undefined);
  const { model } = browser;
  const step = pending
    ? quickStep(pending, {
        identity: panelIdentity(model),
        connected: model.busy ? undefined : connected,
        confirming: confirm.showing,
        takeoverViewer: model.viewer?.takeover === true,
        error: model.error,
      })
    : "wait";
  useEffect(() => {
    if (!pending || step === "wait") return;
    setPending(undefined);
    if (step === "navigate" && connected) browser.navigate(pending.url, connected);
  }, [browser, connected, pending, step]);
  return (url) => {
    const plan = quickLaunchPlan(model, connected);
    if (plan === "none") return;
    if (plan === "navigate" && connected) {
      browser.navigate(url, connected);
      return;
    }
    const identity = panelIdentity(model);
    setPending({ url, identity, takeover: plan === "takeover", error: model.error });
    // "choose": the first open waits for the Machine picked in the chooser.
    if (plan === "open") commands.open(undefined, identity);
    else if (plan === "attach") commands.attach(false);
    else if (plan === "takeover") confirm.ask("takeover");
  };
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
