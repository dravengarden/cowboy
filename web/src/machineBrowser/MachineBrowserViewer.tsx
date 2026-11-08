/** The attached display of one account's Machine browser: noVNC on a stream
 * channel, lease renewal, viewer-owned resizing, remote clipboard capture and
 * the touch keyboard bridge. Loaded lazily so noVNC stays out of the entry
 * chunk. The parent decides what happens when the session ends. */
import { ContentPasteOutlined, KeyboardHideOutlined, KeyboardOutlined } from "@mui/icons-material";
import { Box, Button, Typography } from "@mui/material";
import { type RefObject, useRef } from "react";
import type { BrowserScale, BrowserTarget } from "./api";
import { KEYBOARD_SENTINEL, useTouchKeyboard } from "./touchKeyboardBridge";
import { type ViewerCallbacks, useViewerSession } from "./useViewerSession";
import type { ViewerSession } from "./viewerSession";

export interface MachineBrowserViewerProps extends ViewerCallbacks {
  account: string;
  /** The Machine and instance this viewer attaches to. */
  target: BrowserTarget;
  accountLabel: string;
  machineLabel: string;
  scale: BrowserScale;
  takeover: boolean;
  /** Touch surface: show the keyboard summoner and keep taps from stealing it. */
  touch: boolean;
}

function ViewerControls({
  touch,
  keyboardOpen,
  inputRef,
  sessionRef,
  onNotice,
}: {
  touch: boolean;
  keyboardOpen: boolean;
  inputRef: RefObject<HTMLInputElement | null>;
  sessionRef: RefObject<ViewerSession | undefined>;
  onNotice: (message: string) => void;
}): React.JSX.Element {
  const toggleKeyboard = (): void => {
    const input = inputRef.current;
    if (!input) return;
    if (keyboardOpen) input.blur();
    else input.focus();
  };
  // Read on the press itself: the permission prompt needs the user gesture.
  const pasteFromDevice = (): void => {
    const clipboard = globalThis.navigator?.clipboard;
    if (typeof clipboard?.readText !== "function") {
      onNotice("Reading the clipboard is not available in this browser.");
      return;
    }
    clipboard.readText().then(
      (text) => {
        if (text) sessionRef.current?.paste(text);
        else onNotice("The clipboard is empty.");
      },
      () => onNotice("Clipboard access was denied."),
    );
  };
  const target = touch ? 44 : 32;
  return (
    <Box sx={{ position: "absolute", right: 12, bottom: 12, display: "flex", gap: 1 }}>
      {touch && (
        <Button
          variant="contained"
          size="small"
          onClick={toggleKeyboard}
          startIcon={keyboardOpen ? <KeyboardHideOutlined /> : <KeyboardOutlined />}
          sx={{ minHeight: target, minWidth: target }}
        >
          {keyboardOpen ? "Hide keyboard" : "Keyboard"}
        </Button>
      )}
      <Button
        variant="contained"
        size="small"
        onClick={pasteFromDevice}
        startIcon={<ContentPasteOutlined />}
        sx={{ minHeight: target, minWidth: target }}
      >
        Paste from this device
      </Button>
    </Box>
  );
}

export default function MachineBrowserViewer(props: MachineBrowserViewerProps): React.JSX.Element {
  const { account, target, accountLabel, machineLabel, scale, takeover, touch } = props;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const screenRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const { sessionRef, connected } = useViewerSession({
    containerRef,
    screenRef,
    account,
    target,
    scale,
    takeover,
    callbacks: props,
  });
  const keyboardOpen = useTouchKeyboard(inputRef, sessionRef, touch);
  return (
    <Box
      data-desktop-keys="own"
      data-machine-browser-viewer={account}
      sx={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", gap: 0.5 }}
    >
      <Typography
        variant="caption"
        color="text.secondary"
        sx={{ px: 0.5, overflowWrap: "anywhere" }}
      >
        Browser of{" "}
        <Box component="strong" sx={{ color: "text.primary" }}>
          {accountLabel}
        </Box>{" "}
        on {machineLabel}
      </Typography>
      <Box
        ref={containerRef}
        sx={{
          flex: 1,
          minHeight: 0,
          position: "relative",
          overflow: "hidden",
          borderRadius: 1,
          bgcolor: "#141414",
          touchAction: "none",
        }}
      >
        <Box
          ref={screenRef}
          sx={{ position: "absolute", inset: 0, "& canvas": { outline: "none" } }}
        />
        {!connected && (
          <Typography
            role="status"
            sx={{
              position: "absolute",
              inset: 0,
              display: "grid",
              placeItems: "center",
              color: "#e0e0e0",
              pointerEvents: "none",
              px: 2,
              textAlign: "center",
            }}
          >
            Connecting to the Machine browser…
          </Typography>
        )}
        {touch && (
          <input
            ref={inputRef}
            aria-label="Type into the Machine browser"
            defaultValue={KEYBOARD_SENTINEL}
            autoCapitalize="off"
            autoCorrect="off"
            autoComplete="off"
            spellCheck={false}
            style={{
              position: "absolute",
              left: 0,
              bottom: 0,
              width: 1,
              height: 1,
              opacity: 0,
              border: 0,
              padding: 0,
              pointerEvents: "none",
            }}
          />
        )}
        <ViewerControls
          touch={touch}
          keyboardOpen={keyboardOpen}
          inputRef={inputRef}
          sessionRef={sessionRef}
          onNotice={props.onNotice}
        />
      </Box>
    </Box>
  );
}
