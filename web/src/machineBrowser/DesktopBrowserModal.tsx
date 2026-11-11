/** The Desktop Machine browser: a large centered modal over a dimmed
 * backdrop (the page underneath stays as it was), with one compact header
 * row and the remote frame below. Maximize fills the whole window and is
 * remembered. A backdrop click never closes it. With focus in the remote
 * frame every key, Esc included, belongs to the remote page; Esc closes the
 * modal only from Cowboy's chrome, and the release chord moves focus there.
 * See docs/machine-browser.md, "Desktop panel". */
import { CloseFullscreenRounded, CloseRounded, OpenInFullRounded } from "@mui/icons-material";
import { Box, Button, Dialog, IconButton, Stack, Tooltip } from "@mui/material";
import { type ReactNode, useCallback, useEffect, useId, useRef, useState } from "react";
import { DESKTOP_SHORTCUTS } from "../desktop/commands/workspaceShortcuts";
import { isMac } from "../platform";
import { displayShortcutKey } from "../ShortcutKeycap";
import type { BrowserChord } from "./browserKeys";
import type { BrowserChromeState } from "./browserChrome";
import {
  HEADER_HEIGHT,
  escapeClosesModal,
  isBrowserRelease,
  modalBox,
  readMaximized,
  writeMaximized,
} from "./desktopBrowserLayout";
import { Identity } from "./MachineBrowserAddressBar";
import { BrowserNavigationButtons } from "./MachineBrowserToolbar";

function storage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

/** The remembered Maximize/Restore choice. */
function useMaximized(): [boolean, () => void] {
  const [maximized, setMaximized] = useState(() => readMaximized(storage()));
  const toggle = useCallback(() => {
    setMaximized((old) => {
      writeMaximized(storage(), !old);
      return !old;
    });
  }, []);
  return [maximized, toggle];
}

/** The release chord as the tooltip shows it (`⌘⇧Esc`, `Alt⇧Q`). */
export function releaseLabel(shortcut: string = DESKTOP_SHORTCUTS.browserRelease): string {
  return shortcut.split("+").map(displayShortcutKey).join("");
}

export interface DesktopBrowserModalProps {
  open: boolean;
  onClose: () => void;
  identity: {
    accountLabel: string;
    machineLabel: string;
    status: string;
    shared: boolean;
  };
  chrome: BrowserChromeState;
  onChord: (chord: BrowserChord) => void;
  menu: ReactNode;
  children: ReactNode;
}

/** The compact (40 px) header row: identity and state, navigation, More,
 * Maximize/Restore and Close. */
export function DesktopBrowserHeader({
  labelId,
  identity,
  chrome,
  onChord,
  menu,
  maximized,
  onToggleMaximized,
  onClose,
}: Omit<DesktopBrowserModalProps, "open" | "children"> & {
  labelId: string;
  maximized: boolean;
  onToggleMaximized: () => void;
}): React.JSX.Element {
  const maximizeLabel = maximized ? "Restore" : "Maximize";
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.5}
      data-machine-browser-header
      sx={{ flexShrink: 0, height: HEADER_HEIGHT, px: 1, borderBottom: 1, borderColor: "divider" }}
    >
      <Box id={labelId} sx={{ flex: 1, minWidth: 0, px: 0.5 }}>
        <Identity {...identity} align="left" />
      </Box>
      <BrowserNavigationButtons chrome={chrome} touch={false} onChord={onChord} />
      {menu}
      <Tooltip title={maximized ? "Restore the window size" : "Fill the whole window"}>
        <IconButton
          aria-label={maximizeLabel}
          aria-pressed={maximized}
          onClick={onToggleMaximized}
          sx={{ width: 34, height: 34 }}
        >
          {maximized ? <CloseFullscreenRounded /> : <OpenInFullRounded />}
        </IconButton>
      </Tooltip>
      <Tooltip
        title={`Close (Esc here; inside the page every key is the page's: ${releaseLabel()} returns to this toolbar)`}
      >
        <IconButton aria-label="Close browser" onClick={onClose} sx={{ width: 34, height: 34 }}>
          <CloseRounded />
        </IconButton>
      </Tooltip>
    </Stack>
  );
}

export function DesktopBrowserModal(props: DesktopBrowserModalProps): React.JSX.Element {
  const { open, onClose, children } = props;
  const labelId = useId();
  const [maximized, toggleMaximized] = useMaximized();
  const header = useRef<HTMLDivElement | null>(null);
  // The release chord, before the remote frame's own key handling sees it.
  useEffect(() => {
    if (!open) return undefined;
    const onKey = (event: KeyboardEvent): void => {
      if (!isBrowserRelease(event, DESKTOP_SHORTCUTS.browserRelease, isMac)) return;
      if (escapeClosesModal(document.activeElement)) return;
      event.preventDefault();
      event.stopPropagation();
      header.current?.querySelector<HTMLElement>("button:not([disabled])")?.focus();
    };
    globalThis.addEventListener("keydown", onKey, true);
    return () => globalThis.removeEventListener("keydown", onKey, true);
  }, [open]);
  const box = modalBox(maximized);
  return (
    <Dialog
      open={open}
      aria-labelledby={labelId}
      maxWidth={false}
      onClose={(_event, reason) => {
        // A backdrop click never loses the page; Esc from the remote frame
        // is the page's.
        if (reason === "backdropClick") return;
        if (reason === "escapeKeyDown" && !escapeClosesModal(document.activeElement)) return;
        onClose();
      }}
      slotProps={{
        paper: {
          "data-machine-browser-modal": maximized ? "maximized" : "window",
          sx: {
            ...box,
            m: 0,
            maxWidth: "none",
            maxHeight: "none",
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
          },
        } as object,
      }}
    >
      <Box ref={header}>
        <DesktopBrowserHeader
          {...props}
          labelId={labelId}
          maximized={maximized}
          onToggleMaximized={toggleMaximized}
        />
      </Box>
      <Box sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>{children}</Box>
    </Dialog>
  );
}
