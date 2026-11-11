/** The quick-launch row of the Machine browser panel and its pending link:
 * a tap navigates the viewed browser at once, or opens (attaches to) it and
 * navigates exactly once when it is viewed. A pending link belongs to this
 * panel: a re-bound panel (another browser identity) drops it, so it never
 * reaches another profile. */
import { Button, Stack } from "@mui/material";
import { useEffect, useState } from "react";
import type { ConfirmKind } from "./confirmation";
import type { BrowserCommands } from "./MachineBrowserActions";
import { type QuickLink, quickEntry, quickTarget } from "./quickLinks";
import type { MachineBrowserController } from "./useMachineBrowser";

export function useQuickLaunch(
  browser: MachineBrowserController,
  commands: BrowserCommands,
  onConfirm: (kind: ConfirmKind) => void,
): (url: string) => void {
  const [pending, setPending] = useState<string | undefined>(undefined);
  const target = quickTarget(browser.model);
  useEffect(() => {
    if (!pending || !target) return;
    setPending(undefined);
    browser.navigate(pending, target);
  }, [browser, pending, target]);
  return (url) => {
    const now = quickTarget(browser.model);
    if (now) {
      browser.navigate(url, now);
      return;
    }
    setPending(url);
    const entry = quickEntry(browser.model);
    if (entry === "open") commands.open();
    else if (entry === "attach") commands.attach(false);
    else if (entry === "takeover") onConfirm("takeover");
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
