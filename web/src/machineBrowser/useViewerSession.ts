/** React binding of one viewer session: opens the stream at the screen mode
 * for this device, keeps the latest callbacks without resubscribing, and
 * re-requests the mode on the same session only when it changes (a rotation,
 * a desktop window resize crossing into another mode, or touch input coming
 * or going); the soft keyboard and the panel chrome never resize the remote.
 * The viewer is the only size authority (docs/machine-browser.md, "Viewer
 * (core web)"). */
import { type RefObject, useEffect, useRef, useState } from "react";
import { announceProductAuthSession } from "../productAuthEvents";
import {
  type BrowserStateEvent,
  type BrowserTarget,
  browserErrorText,
  browserReasonText,
  browserStreamUrl,
} from "./api";
import { viewerEnds } from "./browserModel";
import { devicePortrait, onOrientationChange } from "./inputProfile";
import { type ViewerAttachment, attachViewer } from "./viewerAttachment";
import { type ViewerEnd, type ViewerSession, startViewerSession } from "./viewerSession";

export interface ViewerCallbacks {
  onState(state: BrowserStateEvent): void;
  /** The session is over; the owner unmounts the viewer and shows the text. */
  onEnded(message: string): void;
  onRemoteClipboard(text: string | undefined): void;
  onNotice(message: string): void;
}

const RESIZE_DEBOUNCE_MS = 250;

export function viewerEndMessage(end: ViewerEnd): string {
  switch (end.kind) {
    case "auth_required":
      return "Sign in again to continue; the browser stream closed because your Cowboy session needs verification.";
    case "security_failure":
      return end.reason
        ? `The Machine refused the display connection: ${end.reason}`
        : "The Machine refused the display connection.";
    case "credentials_required":
      return "The Machine’s display asked for credentials, which Cowboy does not supply.";
    case "device_proof_failed":
      return "This device could not prove its identity for the stream. Reload and try again.";
    default:
      return end.close.reason
        ? `The browser stream closed (${end.close.reason}).`
        : "The browser stream closed.";
  }
}

function stateEndMessage(state: BrowserStateEvent): string {
  return (
    browserReasonText(state.reason) ??
    (state.state === "stopping" ? "The browser is stopping." : "The browser was hibernated.")
  );
}

export function useViewerSession({
  containerRef,
  screenRef,
  account,
  target,
  touch,
  takeover,
  callbacks,
}: {
  containerRef: RefObject<HTMLDivElement | null>;
  screenRef: RefObject<HTMLDivElement | null>;
  account: string;
  /** The exact Machine and instance to attach to (an open's answer or the
   * state shown); the Controller never substitutes another browser. */
  target: BrowserTarget;
  /** Touch input: the mode follows the device's orientation. */
  touch: boolean;
  takeover: boolean;
  callbacks: ViewerCallbacks;
}): { sessionRef: RefObject<ViewerSession | undefined>; connected: boolean } {
  const sessionRef = useRef<ViewerSession | undefined>(undefined);
  const attachmentRef = useRef<ViewerAttachment<ViewerSession> | undefined>(undefined);
  const latest = useRef(callbacks);
  latest.current = callbacks;
  // The input mode is read when the session opens and followed by resizing
  // it; it is not part of the connection's lifetime.
  const touchRef = useRef(touch);
  touchRef.current = touch;
  const [connected, setConnected] = useState(false);
  const machineId = target.machine_id;
  const { instance_id, incarnation } = target.instance;
  useEffect(() => {
    const container = containerRef.current;
    const target = screenRef.current;
    if (!container || !target) return undefined;
    const attachment = attachViewer({
      frame: () => container.getBoundingClientRect(),
      portrait: devicePortrait,
      touch: touchRef.current,
      debounceMs: RESIZE_DEBOUNCE_MS,
      watch: (changed) => {
        const observer = new ResizeObserver(changed);
        observer.observe(container);
        const stopOrientation = onOrientationChange(changed);
        return () => {
          observer.disconnect();
          stopOrientation();
        };
      },
      open: (viewport) =>
        startViewerSession({
          target,
          url: browserStreamUrl(
            account,
            { machine_id: machineId, instance: { account_id: account, instance_id, incarnation } },
            viewport,
            takeover,
          ),
          // `attachment` is assigned before the socket can open.
          onStreamOpen: () => attachment.streamOpened(),
          onState: (state) => {
            latest.current.onState(state);
            if (viewerEnds(state)) latest.current.onEnded(stateEndMessage(state));
          },
          onStreamError: (code) => latest.current.onNotice(browserErrorText(code)),
          onAuthSession: announceProductAuthSession,
          onConnected: () => setConnected(true),
          onClipboard: (text) => latest.current.onRemoteClipboard(text),
          onEnded: (end) => latest.current.onEnded(viewerEndMessage(end)),
        }),
    });
    attachmentRef.current = attachment;
    sessionRef.current = attachment.session;
    return () => {
      attachment.close();
      attachmentRef.current = undefined;
      sessionRef.current = undefined;
      setConnected(false);
      latest.current.onRemoteClipboard(undefined);
    };
  }, [account, containerRef, incarnation, instance_id, machineId, screenRef, takeover]);
  useEffect(() => {
    attachmentRef.current?.setTouch(touch);
  }, [touch]);
  return { sessionRef, connected };
}
