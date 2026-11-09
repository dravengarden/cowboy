/** The lifetime of one attached viewer apart from its input mode: the
 * session opens once at the screen mode for this device, and every later
 * mode change (a rotation, a desktop resize crossing into another mode, or
 * touch input appearing or going away with a tablet's trackpad) resizes that
 * same session over its resize channel. Reattaching for a mode change would
 * race the Machine's detach acknowledgement (`browser_attached`). A mode
 * change before the stream opens is kept and sent once it does; only a
 * resize that went out counts as sent.
 * Framework-free so it is testable without React or noVNC. */
import { type BrowserViewport, browserViewport, viewportChanged } from "./api";

export interface ResizableSession {
  /** Whether the resize went out. */
  resize(width: number, height: number): boolean;
  close(): void;
}

export interface ViewerAttachmentOptions<Session extends ResizableSession> {
  /** The frame's CSS size now. */
  frame(): { width: number; height: number };
  /** Whether the device is held upright now. */
  portrait(): boolean;
  touch: boolean;
  /** Open the session at its first viewport. */
  open(viewport: BrowserViewport): Session;
  /** Call `changed` on every frame resize or rotation; returns the
   * unsubscribe. */
  watch(changed: () => void): () => void;
  debounceMs: number;
}

export interface ViewerAttachment<Session extends ResizableSession> {
  session: Session;
  /** Switch between touch and pointer input; resizes the session at once
   * when the mode differs. */
  setTouch(touch: boolean): void;
  /** The stream opened: send the mode it does not have yet, if any. */
  streamOpened(): void;
  close(): void;
}

export function attachViewer<Session extends ResizableSession>(
  options: ViewerAttachmentOptions<Session>,
): ViewerAttachment<Session> {
  let touch = options.touch;
  const viewportNow = (): BrowserViewport => {
    const { width, height } = options.frame();
    return browserViewport(width, height, { touch, portrait: options.portrait() });
  };
  /** The mode the remote has or is opening with (the stream URL's). */
  let sent = viewportNow();
  const session = options.open(sent);
  const send = (next: BrowserViewport): void => {
    if (!viewportChanged(sent, next)) return;
    if (session.resize(next.width, next.height)) sent = next;
  };
  const apply = (): void => send(viewportNow());
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stopWatching = options.watch(() => {
    clearTimeout(timer);
    timer = setTimeout(apply, options.debounceMs);
  });
  return {
    session,
    setTouch: (next) => {
      if (next === touch) return;
      touch = next;
      apply();
    },
    streamOpened: apply,
    close: () => {
      clearTimeout(timer);
      stopWatching();
      session.close();
    },
  };
}
