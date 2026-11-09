/** The attached display of one account's Machine browser: noVNC on a stream
 * channel, lease renewal, viewer-owned resizing, remote clipboard capture and
 * the touch keyboard bridge. The framebuffer fills the frame the panel gives
 * it; the browser chrome drives it through the `ViewerControls` this viewer
 * publishes once connected (bound to its account, Machine and instance).
 * Loaded lazily so noVNC stays out of the entry chunk. The parent decides
 * what happens when the session ends. */
import { Box, Typography } from "@mui/material";
import { useEffect, useRef } from "react";
import type { BrowserScale, BrowserTarget } from "./api";
import type { ViewerControls } from "./browserChrome";
import { KEYBOARD_SENTINEL, useTouchKeyboard } from "./touchKeyboardBridge";
import { type ViewerCallbacks, useViewerSession } from "./useViewerSession";

export interface MachineBrowserViewerProps extends ViewerCallbacks {
  account: string;
  /** The Machine and instance this viewer attaches to. */
  target: BrowserTarget;
  scale: BrowserScale;
  takeover: boolean;
  /** Touch surface: bridge the on-screen keyboard and keep taps from stealing it. */
  touch: boolean;
  /** This viewer's key controls while connected, `undefined` once gone. */
  onControls(controls: ViewerControls | undefined): void;
  onKeyboard(open: boolean): void;
}

const HIDDEN_INPUT_STYLE = {
  position: "absolute",
  left: 0,
  bottom: 0,
  width: 1,
  height: 1,
  opacity: 0,
  border: 0,
  padding: 0,
  pointerEvents: "none",
} as const;

export default function MachineBrowserViewer(props: MachineBrowserViewerProps): React.JSX.Element {
  const { account, target, scale, takeover, touch } = props;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const screenRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const latest = useRef(props);
  latest.current = props;
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
  // The viewer is keyed by its generation: its target never changes while
  // it is mounted, so the controls it publishes stay bound to that browser.
  useEffect(() => {
    if (!connected) return undefined;
    latest.current.onControls({
      account,
      target: latest.current.target,
      chord: (chord) => sessionRef.current?.sendChord(chord),
      toggleKeyboard: () => {
        const input = inputRef.current;
        if (!input) return;
        if (globalThis.document?.activeElement === input) input.blur();
        else input.focus();
      },
      paste: (text) => sessionRef.current?.paste(text),
    });
    return () => latest.current.onControls(undefined);
  }, [account, connected, sessionRef]);
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
      {touch && (
        <input
          ref={inputRef}
          aria-label="Type into the Machine browser"
          defaultValue={KEYBOARD_SENTINEL}
          autoCapitalize="off"
          autoCorrect="off"
          autoComplete="off"
          spellCheck={false}
          style={HIDDEN_INPUT_STYLE}
        />
      )}
    </Box>
  );
}
