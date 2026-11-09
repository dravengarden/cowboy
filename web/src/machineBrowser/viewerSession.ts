/** One attached display: the stream socket signed with the device proof,
 * noVNC's RFB client bound to it, the 10 s lease renewal and the input
 * helpers the viewer component calls. Framework-free so the component stays
 * small; noVNC is only ever loaded through this module's lazy chunk. */
import RFB from "@novnc/novnc";
import { browserDeviceProtocol } from "../browserDevice";
import type { BrowserStateEvent } from "./api";
import {
  BROWSER_STREAM_SUBPROTOCOL,
  type BrowserStream,
  LEASE_RENEWAL_MS,
  type StreamClose,
  createBrowserStream,
} from "./stream";
import { type BrowserChord, chordStrokes } from "./browserKeys";
import type { KeyAction } from "./touchKeyboard";

export type ViewerEnd =
  | { kind: "closed"; close: StreamClose }
  | { kind: "auth_required" }
  | { kind: "security_failure"; reason: string | undefined }
  | { kind: "credentials_required" }
  | { kind: "device_proof_failed" };

export interface ViewerSessionOptions {
  /** The element noVNC fills with its screen. */
  target: HTMLElement;
  /** The final stream URL including every query parameter. */
  url: string;
  onState(state: BrowserStateEvent): void;
  onStreamError(code: string): void;
  onAuthSession(session: unknown): void;
  onConnected(): void;
  onClipboard(text: string): void;
  onEnded(end: ViewerEnd): void;
}

export interface ViewerSession {
  resize(width: number, height: number): void;
  paste(text: string): void;
  sendKey(action: KeyAction): void;
  /** Press a browser navigation chord (`chordStrokes`) on the remote. */
  sendChord(chord: BrowserChord): void;
  /** Whether a tap or click on the screen moves keyboard focus to it. */
  setFocusOnClick(value: boolean): void;
  focus(): void;
  close(): void;
}

export function startViewerSession(options: ViewerSessionOptions): ViewerSession {
  let ended = false;
  let closing = false;
  let rfb: RFB | undefined;
  let stream: BrowserStream | undefined;
  let lease: ReturnType<typeof setInterval> | undefined;
  let focusOnClick = true;
  const end = (value: ViewerEnd): void => {
    if (ended) return;
    ended = true;
    clearInterval(lease);
    if (!closing) options.onEnded(value);
  };
  // The proof signs the final URL; the Controller validates it on upgrade.
  browserDeviceProtocol(options.url).then(
    (deviceProtocol) => {
      if (closing) return;
      const socket = new WebSocket(options.url, [BROWSER_STREAM_SUBPROTOCOL, deviceProtocol]);
      stream = createBrowserStream(socket, {
        onControl: (message) => {
          switch (message.type) {
            case "state":
              options.onState(message.state);
              break;
            case "error":
              options.onStreamError(message.code);
              break;
            case "auth_session":
              options.onAuthSession(message.session);
              break;
            default:
              break;
          }
        },
        onClose: (close) =>
          end(close.authRequired ? { kind: "auth_required" } : { kind: "closed", close }),
      });
      rfb = new RFB(options.target, stream.channel, { shared: true });
      rfb.scaleViewport = true;
      rfb.clipViewport = false;
      rfb.resizeSession = false;
      rfb.viewOnly = false;
      rfb.showDotCursor = false;
      rfb.focusOnClick = focusOnClick;
      rfb.background = "transparent";
      rfb.qualityLevel = 6;
      rfb.compressionLevel = 2;
      rfb.addEventListener("connect", () => options.onConnected());
      rfb.addEventListener("clipboard", (event) => options.onClipboard(event.detail.text));
      rfb.addEventListener("securityfailure", (event) =>
        end({ kind: "security_failure", reason: event.detail.reason }),
      );
      rfb.addEventListener("credentialsrequired", () => end({ kind: "credentials_required" }));
      // The socket's close is the end of record; noVNC's own disconnect follows it.
      lease = setInterval(() => {
        stream?.sendLease();
      }, LEASE_RENEWAL_MS);
    },
    () => end({ kind: "device_proof_failed" }),
  );
  return {
    resize: (width, height) => {
      stream?.sendResize(width, height);
    },
    paste: (text) => rfb?.clipboardPasteFrom(text),
    sendKey: (action) => rfb?.sendKey(action.keysym, action.code),
    sendChord: (chord) => {
      const client = rfb;
      if (!client) return;
      for (const stroke of chordStrokes(chord))
        client.sendKey(stroke.keysym, stroke.code, stroke.down);
    },
    setFocusOnClick: (value) => {
      focusOnClick = value;
      if (rfb) rfb.focusOnClick = value;
    },
    focus: () => rfb?.focus({ preventScroll: true }),
    close: () => {
      closing = true;
      ended = true;
      clearInterval(lease);
      rfb?.disconnect();
      stream?.close();
    },
  };
}
