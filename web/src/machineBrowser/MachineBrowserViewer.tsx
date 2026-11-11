/** The attached display of one account's Machine browser: noVNC on a stream
 * channel, lease renewal, the fixed screen mode, remote clipboard capture
 * and, on touch surfaces, the neko-style keyboard overlay. The framebuffer is
 * scaled to fit and letterboxed in the frame the panel gives it; the browser
 * chrome drives it through the `ViewerControls` this viewer
 * publishes once connected (bound to its account, Machine and instance).
 * Loaded lazily so noVNC stays out of the entry chunk. The parent decides
 * what happens when the session ends. */
import { Box, Typography } from "@mui/material";
import { useEffect, useRef } from "react";
import type { BrowserTarget } from "./api";
import type { ViewerControls } from "./browserChrome";
import { useTouchKeyboard } from "./touchKeyboardBridge";
import { type ViewerCallbacks, useViewerSession } from "./useViewerSession";

export interface MachineBrowserViewerProps extends ViewerCallbacks {
  account: string;
  /** The Machine and instance this viewer attaches to. */
  target: BrowserTarget;
  takeover: boolean;
  /** Touch input: the keyboard overlay and the device-orientation mode. */
  touch: boolean;
  /** This viewer's key controls while connected, `undefined` once gone. */
  onControls(controls: ViewerControls | undefined): void;
  onKeyboard(open: boolean): void;
}

/** neko's overlay: full size so iOS never scrolls to reveal it, 16 px so it
 * never zooms, invisible, and transparent to pointers so taps, long presses
 * and two-finger scrolls still reach noVNC's canvas below while it holds
 * focus (only the Keyboard button focuses it). */
const KEYBOARD_OVERLAY_STYLE = {
  position: "absolute",
  inset: 0,
  zIndex: 1,
  width: "100%",
  height: "100%",
  margin: 0,
  padding: 0,
  border: 0,
  outline: 0,
  resize: "none",
  overflow: "hidden",
  fontSize: 16,
  color: "transparent",
  caretColor: "transparent",
  background: "transparent",
  pointerEvents: "none",
} as const;

export default function MachineBrowserViewer(props: MachineBrowserViewerProps): React.JSX.Element {
  const { account, target, takeover, touch } = props;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const screenRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);
  const latest = useRef(props);
  latest.current = props;
  const { sessionRef, connected, attachmentId } = useViewerSession({
    containerRef,
    screenRef,
    account,
    target,
    touch,
    takeover,
    callbacks: props,
  });
  const { open: keyboardOpen, overlay } = useTouchKeyboard(inputRef, sessionRef, touch);
  // The viewer is keyed by its generation: its target never changes while
  // it is mounted, so the controls it publishes stay bound to that browser.
  useEffect(() => {
    if (!connected) return undefined;
    latest.current.onControls({
      account,
      target: latest.current.target,
      attachmentId,
      chord: (chord) => sessionRef.current?.sendChord(chord),
      // Synchronous inside the Keyboard tap, or iOS shows no keyboard.
      toggleKeyboard: () => {
        const input = inputRef.current;
        if (!input) return;
        if (globalThis.document?.activeElement === input) input.blur();
        else input.focus({ preventScroll: true });
      },
      paste: (text) => sessionRef.current?.pasteText(text),
    });
    return () => latest.current.onControls(undefined);
  }, [account, attachmentId, connected, sessionRef]);
  useEffect(() => {
    latest.current.onKeyboard(keyboardOpen);
  }, [keyboardOpen]);
  useEffect(() => () => latest.current.onKeyboard(false), []);
  return (
    <Box
      ref={containerRef}
      data-desktop-keys="own"
      data-machine-browser-viewer={account}
      sx={{
        flex: 1,
        minHeight: 0,
        position: "relative",
        overflow: "hidden",
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
      {overlay && (
        <textarea
          ref={inputRef}
          aria-label="Type into the Machine browser"
          data-machine-browser-keyboard
          data-gramm="false"
          inputMode="text"
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          tabIndex={-1}
          style={KEYBOARD_OVERLAY_STYLE}
        />
      )}
    </Box>
  );
}
