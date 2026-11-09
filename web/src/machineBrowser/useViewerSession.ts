/** React binding of one viewer session: opens the stream at the screen mode
 * for this device, keeps the latest callbacks without resubscribing, and
 * re-requests the mode only when it changes (a rotation, or a desktop window
 * resize crossing into another mode); the soft keyboard and the panel chrome
 * never resize the remote. The viewer is the only size authority
 * (docs/machine-browser.md, "Viewer (core web)"). */
import { type RefObject, useEffect, useRef, useState } from "react";
import { announceProductAuthSession } from "../productAuthEvents";
import {
  type BrowserStateEvent,
  type BrowserTarget,
  browserErrorText,
  browserReasonText,
  browserStreamUrl,
  browserViewport,
  viewportChanged,
} from "./api";
import { viewerEnds } from "./browserModel";
import { devicePortrait, onOrientationChange } from "./inputProfile";
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
  const latest = useRef(callbacks);
  latest.current = callbacks;
  const [connected, setConnected] = useState(false);
  const machineId = target.machine_id;
  const { instance_id, incarnation } = target.instance;
  useEffect(() => {
    const container = containerRef.current;
    const target = screenRef.current;
    if (!container || !target) return undefined;
    const viewportNow = () => {
      const rect = container.getBoundingClientRect();
      return browserViewport(rect.width, rect.height, { touch, portrait: devicePortrait() });
    };
    let sent = viewportNow();
    const session = startViewerSession({
      target,
      url: browserStreamUrl(
        account,
        { machine_id: machineId, instance: { account_id: account, instance_id, incarnation } },
        sent,
        takeover,
      ),
      onState: (state) => {
        latest.current.onState(state);
        if (viewerEnds(state)) latest.current.onEnded(stateEndMessage(state));
      },
      onStreamError: (code) => latest.current.onNotice(browserErrorText(code)),
      onAuthSession: announceProductAuthSession,
      onConnected: () => setConnected(true),
      onClipboard: (text) => latest.current.onRemoteClipboard(text),
      onEnded: (end) => latest.current.onEnded(viewerEndMessage(end)),
    });
    sessionRef.current = session;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const settle = (): void => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const next = viewportNow();
        if (!viewportChanged(sent, next)) return;
        sent = next;
        session.resize(next.width, next.height);
      }, RESIZE_DEBOUNCE_MS);
    };
    const observer = new ResizeObserver(settle);
    observer.observe(container);
    const stopOrientation = onOrientationChange(settle);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
      stopOrientation();
      session.close();
      sessionRef.current = undefined;
      latest.current.onRemoteClipboard(undefined);
    };
  }, [account, containerRef, incarnation, instance_id, machineId, screenRef, takeover, touch]);
  return { sessionRef, connected };
}
