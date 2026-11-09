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
import {
  type NativeTouch,
  type RfbInternals,
  type RfbMessages,
  attachNativeTouch,
  hookEncodings,
} from "./nativeTouch";
import type { KeyAction, KeyboardSink } from "./touchKeyboard";

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
  /** The stream accepts resizes from now on. */
  onStreamOpen(): void;
  onStreamError(code: string): void;
  onAuthSession(session: unknown): void;
  onConnected(): void;
  onClipboard(text: string): void;
  onEnded(end: ViewerEnd): void;
}

export interface ViewerSession extends KeyboardSink {
  /** Whether the resize went out; it cannot before the stream opens. */
  resize(width: number, height: number): boolean;
  sendKey(action: KeyAction): void;
  /** Press a browser chord (`chordStrokes`) on the remote. */
  sendChord(chord: BrowserChord): void;
  focus(): void;
  close(): void;
}

function sendChord(client: RFB | undefined, chord: BrowserChord): void {
  if (!client) return;
  for (const stroke of chordStrokes(chord)) client.sendKey(stroke.keysym, stroke.code, stroke.down);
}

/** Native touch on this RFB client: requested in its SetEncodings, used
 * only once the display server acknowledges it (`nativeTouch.ts`). */
function startNativeTouch(rfb: RFB, target: HTMLElement, focus: () => void): NativeTouch {
  hookEncodings((RFB as unknown as { messages: RfbMessages }).messages);
  return attachNativeTouch(rfb as unknown as RfbInternals, {
    surface: target,
    canvas: () => target.querySelector("canvas"),
    page: globalThis,
    visibility: globalThis.document,
    onContact: focus,
  });
}

export function startViewerSession(options: ViewerSessionOptions): ViewerSession {
  let ended = false;
  let closing = false;
  let rfb: RFB | undefined;
  let stream: BrowserStream | undefined;
  let touch: NativeTouch | undefined;
  let lease: ReturnType<typeof setInterval> | undefined;
  let focusOnClick = true;
  const end = (value: ViewerEnd): void => {
    if (ended) return;
    ended = true;
    clearInterval(lease);
    touch?.close();
    if (!closing) options.onEnded(value);
  };
  // The proof signs the final URL; the Controller validates it on upgrade.
  browserDeviceProtocol(options.url).then(
    (deviceProtocol) => {
      if (closing) return;
      const socket = new WebSocket(options.url, [BROWSER_STREAM_SUBPROTOCOL, deviceProtocol]);
      stream = createBrowserStream(socket, {
        onOpen: () => {
          if (!closing) options.onStreamOpen();
        },
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
      const client = new RFB(options.target, stream.channel, { shared: true });
      rfb = client;
      // A contact focuses the canvas as noVNC's own tap would.
      touch = startNativeTouch(client, options.target, () => {
        if (focusOnClick) client.focus({ preventScroll: true });
      });
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
      rfb.addEventListener("disconnect", () => touch?.close());
      // The socket's close is the end of record; noVNC's own disconnect follows it.
      lease = setInterval(() => {
        stream?.sendLease();
      }, LEASE_RENEWAL_MS);
    },
    () => end({ kind: "device_proof_failed" }),
  );
  return {
    resize: (width, height) => stream?.sendResize(width, height) ?? false,
    sendKey: (action) => rfb?.sendKey(action.keysym, action.code),
    sendChord: (chord) => sendChord(rfb, chord),
    // ClientCutText and the key events travel in order on one RFB stream,
    // so the remote clipboard holds `text` before Ctrl+V is pressed.
    pasteText: (text) => {
      const client = rfb;
      if (!client) return;
      client.clipboardPasteFrom(text);
      sendChord(client, "paste");
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
      touch?.close();
      rfb?.disconnect();
      stream?.close();
    },
  };
}
