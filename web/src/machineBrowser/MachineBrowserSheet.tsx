/** The panel an account card opens for its Machine browser: a slim identity
 * row on top and the remote page filling the frame; the remote Chromium shows
 * its own address bar and tabs. Mobile is an edge-to-edge cover with a bottom
 * toolbar (Back, Forward, Reload, Keyboard, menu); Desktop is a large
 * workbench with the menu in its top row and an explicit full-screen mode.
 * See docs/machine-browser.md, "Viewer (core web)". */
import { Box } from "@mui/material";
import { type RefObject, useEffect, useRef, useState } from "react";
import { Sheet } from "../Sheet";
import { copyText } from "../clipboard";
import { useStoreSelector } from "../store";
import { useSurfaceProfile } from "../surface/SurfaceProfile";
import { type BrowserScale, type BrowserViewport, browserStateLabel, browserViewport } from "./api";
import {
  type BrowserChromeState,
  type ViewerControls,
  chromeState,
  controlsFor,
  retryOffered,
} from "./browserChrome";
import { type BrowserModel, autoEntry, linkEntry, linkTarget } from "./browserModel";
import type { ConfirmKind } from "./confirmation";
import { type BrowserCommands, useBrowserConfirm } from "./MachineBrowserActions";
import { DesktopBrowserTopBar, MobileBrowserTopBar } from "./MachineBrowserAddressBar";
import { BrowserFrame, type BrowserFrameProps } from "./MachineBrowserFrame";
import { BrowserMenu, type BrowserMenuItem, MobileBrowserToolbar } from "./MachineBrowserToolbar";
import { useBrowserFullscreen } from "./useBrowserFullscreen";
import { type MachineBrowserController, useMachineBrowser } from "./useMachineBrowser";

export interface MachineBrowserSheetProps {
  open: boolean;
  onClose: () => void;
  account: { id: string; label: string; browserEnabled: boolean };
  owner: string;
  dataset: string;
  /** Open or attach as soon as the state is known. */
  autoOpen?: boolean | undefined;
  /** A link to show once in the account's ordinary tab browser on
   * `machine` (for example a Provider sign-in's verification URL): opens
   * are pinned to that Machine and the browser navigates there once it
   * runs. Nothing flows back: the panel never reads or fills anything for
   * the caller. */
  link?: { url: string; machine: string } | undefined;
  onEnableBrowser?: (() => void) | undefined;
  /** The enablement write is pending elsewhere. */
  enabling?: boolean | undefined;
}

/** Commands over the controller; the viewport is measured from the frame
 * at the moment of the request. `pinned` keeps every open on that Machine. */
function browserCommands(
  browser: MachineBrowserController,
  areaRef: RefObject<HTMLDivElement | null>,
  scale: BrowserScale,
  pinned: string | undefined,
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
    open: (machine) => browser.open(viewportNow(), pinned ?? machine),
    attach: (takeover, target) => dispatch({ type: "attach", takeover, target }),
    detach: () => dispatch({ type: "detach" }),
    hibernate: browser.hibernate,
    snapshot: browser.snapshot,
    reset: browser.reset,
    restart: (target) => browser.restart(viewportNow(), target),
    refresh: browser.refresh,
    dismiss: () => dispatch({ type: "dismiss" }),
    notice: (message) => dispatch({ type: "notice", message }),
  };
}

/** Open or attach once when asked to, and show a link once in the running
 * browser on its Machine only. */
function useAutoEntry(
  props: MachineBrowserSheetProps,
  browser: MachineBrowserController,
  commands: BrowserCommands,
): void {
  const { open, link, autoOpen = false } = props;
  const { model } = browser;
  const state = model.read?.state ?? null;
  const commandsRef = useRef(commands);
  commandsRef.current = commands;
  const autoOpened = useRef(false);
  const readyToAutoOpen =
    (autoOpen || link !== undefined) &&
    open &&
    model.loaded &&
    !model.busy &&
    !model.read?.admission;
  useEffect(() => {
    if (!readyToAutoOpen || autoOpened.current) return;
    autoOpened.current = true;
    const entry = link ? linkEntry(model.read, link.machine) : autoEntry(state?.state);
    if (entry === "attach") commandsRef.current.attach(false);
    else if (entry === "open") commandsRef.current.open();
  }, [readyToAutoOpen, link, model.read, state]);
  const linked = useRef(false);
  const shownLink = link && !model.busy ? linkTarget(model, link.machine) : undefined;
  useEffect(() => {
    if (!link || !shownLink || linked.current) return;
    linked.current = true;
    browser.navigate(link.url, shownLink);
  }, [browser, link, shownLink]);
}

/** Read the device clipboard on the press itself (the permission prompt
 * needs the user gesture) and paste it into this viewer's session. */
function pasteFromDevice(
  controls: ViewerControls | undefined,
  notice: (text: string) => void,
): void {
  const clipboard = globalThis.navigator?.clipboard;
  if (!controls) return;
  if (typeof clipboard?.readText !== "function") {
    notice("Reading the clipboard is not available in this browser.");
    return;
  }
  clipboard.readText().then(
    (text) => {
      if (text) controls.paste(text);
      else notice("The clipboard is empty.");
    },
    () => notice("Clipboard access was denied."),
  );
}

function browserMenuItems({
  model,
  chrome,
  controls,
  commands,
  ask,
  onClose,
}: {
  model: BrowserModel;
  chrome: BrowserChromeState;
  controls: ViewerControls | undefined;
  commands: BrowserCommands;
  ask: (kind: ConfirmKind) => void;
  onClose: () => void;
}): BrowserMenuItem[] {
  const remote = model.remoteClipboard;
  const items: (BrowserMenuItem | false)[] = [
    {
      key: "hibernate",
      label: "Hibernate…",
      disabled: !chrome.hibernate,
      onSelect: () => ask("hibernate"),
    },
    {
      key: "snapshot",
      label: "Save snapshot",
      disabled: !chrome.snapshot,
      onSelect: commands.snapshot,
    },
    chrome.restart && {
      key: "restart",
      label: "Restart for this screen…",
      onSelect: () => ask("restart"),
    },
    model.viewer !== undefined && {
      key: "detach",
      label: "Stop viewing",
      disabled: !chrome.detach,
      onSelect: commands.detach,
    },
    {
      key: "reset",
      label: "Reset profile…",
      destructive: true,
      disabled: !chrome.reset,
      onSelect: () => ask("reset"),
    },
    {
      key: "paste",
      label: "Paste from this device",
      group: true,
      disabled: !chrome.paste,
      onSelect: () => pasteFromDevice(controls, commands.notice),
    },
    {
      key: "copy",
      label:
        remote === undefined
          ? "Copy remote clipboard to this device"
          : `Copy remote clipboard (${[...remote].length} characters) to this device`,
      disabled: !chrome.copy,
      onSelect: () => {
        if (remote === undefined) return;
        copyText(remote).then((copied) =>
          commands.notice(
            copied ? "Remote clipboard copied to this device." : "Could not copy on this device.",
          ),
        );
      },
    },
    {
      key: "refresh",
      label: "Refresh state",
      group: true,
      disabled: !chrome.refresh,
      onSelect: commands.refresh,
    },
    { key: "close", label: "Close", onSelect: onClose },
  ];
  return items.filter((item): item is BrowserMenuItem => item !== false);
}

/** The per-account chrome state: viewer controls bound to this account's
 * shown browser and the keyboard. */
function useBrowserChrome(account: string, model: BrowserModel, scale: BrowserScale) {
  const [published, setPublished] = useState<ViewerControls | undefined>(undefined);
  const [keyboardOpen, setKeyboardOpen] = useState(false);
  const controls = controlsFor(published, account, model);
  return {
    controls,
    chrome: chromeState(model, controls !== undefined, scale),
    keyboardOpen: controls !== undefined && keyboardOpen,
    setPublished,
    setKeyboardOpen,
  };
}

export function MachineBrowserSheet(props: MachineBrowserSheetProps): React.JSX.Element {
  const { open, onClose, account, owner, dataset, link, enabling = false } = props;
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
  });
  const { model } = browser;
  const areaRef = useRef<HTMLDivElement | null>(null);
  const state = model.read?.state ?? null;
  const machineId = model.read?.machine_id ?? model.read?.placements[0]?.machine_id ?? null;
  const machineLabel = machineId
    ? machines.find((machine) => machine.id === machineId)?.display_name || machineId
    : "no Machine yet";
  const commands = browserCommands(browser, areaRef, scale, link?.machine);
  useAutoEntry(props, browser, commands);
  const ui = useBrowserChrome(account.id, model, scale);
  const confirm = useBrowserConfirm(model, state, commands);
  const full = useBrowserFullscreen(desktop && open);
  const close = desktop ? full.guardClose(onClose) : onClose;
  const status = model.loaded ? browserStateLabel(state?.state) : "Loading";
  const menu = (
    <BrowserMenu
      touch={!desktop}
      upward={!desktop}
      items={browserMenuItems({
        model,
        chrome: ui.chrome,
        controls: ui.controls,
        commands,
        ask: confirm.ask,
        onClose,
      })}
    />
  );
  const chord = (value: Parameters<ViewerControls["chord"]>[0]): void => ui.controls?.chord(value);
  const frameProps: BrowserFrameProps = {
    areaRef,
    browser,
    account,
    machineLabel,
    scale,
    touch: !desktop,
    link: link !== undefined,
    pinned: link !== undefined,
    retry: retryOffered(model, link !== undefined),
    commands,
    enabling,
    onEnableBrowser: props.onEnableBrowser,
    onConfirm: confirm.ask,
    onControls: ui.setPublished,
    onKeyboard: ui.setKeyboardOpen,
  };
  const identity = { accountLabel: account.label, machineLabel, status };
  const title = `Browser of ${account.label}`;
  if (desktop) {
    return (
      <Sheet
        open={open}
        onClose={close}
        title={title}
        desktopMaxWidth={1240}
        desktopFill
        desktopFullScreen={full.fullscreen}
      >
        <Box
          data-machine-browser-sheet={account.id}
          sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", mx: -3, mt: -2.5 }}
        >
          <DesktopBrowserTopBar
            {...identity}
            menu={menu}
            fullscreen={full.fullscreen}
            onToggleFullscreen={full.toggle}
            onClose={onClose}
          />
          <BrowserFrame {...frameProps} />
        </Box>
        {confirm.dialog}
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
      hideMobileTitle
      mobileDismiss="none"
    >
      <Box
        data-machine-browser-sheet={account.id}
        sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}
      >
        <MobileBrowserTopBar {...identity} onClose={onClose} />
        <BrowserFrame {...frameProps} />
        <MobileBrowserToolbar
          chrome={ui.chrome}
          keyboardOpen={ui.keyboardOpen}
          onChord={chord}
          onKeyboard={() => ui.controls?.toggleKeyboard()}
          menu={menu}
        />
      </Box>
      {confirm.dialog}
    </Sheet>
  );
}
