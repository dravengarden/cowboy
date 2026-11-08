/** React binding of one viewer session: opens the stream for the measured
 * container, keeps the latest callbacks without resubscribing, and turns
 * settled container resizes into bounded resize requests. The viewer is the
 * only size authority (docs/machine-browser.md, "Viewer (core web)"). */
import { type RefObject, useEffect, useRef, useState } from "react";
import { announceProductAuthSession } from "../productAuthEvents";
import {
  type BrowserScale,
  type BrowserStateEvent,
  browserErrorText,
  browserReasonText,
  browserStreamUrl,
  browserViewport,
  sizeChanged,
} from "./api";
import { viewerEnds } from "./browserModel";
import { type ViewerEnd, type ViewerSession, startViewerSession } from "./viewerSession";

export interface ViewerCallbacks {
  onState(state: BrowserStateEvent): void;
  /** The session is over; the owner unmounts the viewer and shows the text. */
  onEnded(message: string): void;
  onRemoteClipboard(text: string | undefined): void;
  onNotice(message: string): void;
}

const RESIZE_DEBOUNCE_MS = 250;
const RESIZE_THRESHOLD_PX = 16;

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
  scale,
  takeover,
  callbacks,
}: {
  containerRef: RefObject<HTMLDivElement | null>;
  screenRef: RefObject<HTMLDivElement | null>;
  account: string;
  scale: BrowserScale;
  takeover: boolean;
  callbacks: ViewerCallbacks;
}): { sessionRef: RefObject<ViewerSession | undefined>; connected: boolean } {
  const sessionRef = useRef<ViewerSession | undefined>(undefined);
  const latest = useRef(callbacks);
  latest.current = callbacks;
  const [connected, setConnected] = useState(false);
  useEffect(() => {
    const container = containerRef.current;
    const target = screenRef.current;
    if (!container || !target) return undefined;
    const rect = container.getBoundingClientRect();
    let sent = { width: rect.width, height: rect.height };
    const session = startViewerSession({
      target,
      url: browserStreamUrl(account, browserViewport(rect.width, rect.height, scale), takeover),
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
    const observer = new ResizeObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(() => {
        const next = container.getBoundingClientRect();
        if (!sizeChanged(sent, next, RESIZE_THRESHOLD_PX)) return;
        sent = { width: next.width, height: next.height };
        const viewport = browserViewport(next.width, next.height, scale);
        session.resize(viewport.width, viewport.height);
      }, RESIZE_DEBOUNCE_MS);
    });
    observer.observe(container);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
      session.close();
      sessionRef.current = undefined;
      latest.current.onRemoteClipboard(undefined);
    };
  }, [account, containerRef, scale, screenRef, takeover]);
  return { sessionRef, connected };
}
