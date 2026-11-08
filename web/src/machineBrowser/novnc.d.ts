// The subset of noVNC's RFB core the Machine browser viewer uses. noVNC 1.7
// publishes only `./core/rfb.js` through its package root export and ships no
// type declarations; keep this file to what the viewer calls.
declare module "@novnc/novnc" {
  /** A WebSocket-shaped channel `RFB` accepts instead of a URL. The property
   * names are exactly the ones noVNC's `Websock.attach` checks for. */
  export interface RfbChannel {
    binaryType: BinaryType;
    readonly protocol: string;
    readonly readyState: number;
    onopen: ((event: Event) => void) | null;
    onmessage: ((event: { data: ArrayBuffer }) => void) | null;
    onclose: ((event: { code: number; reason: string; wasClean: boolean }) => void) | null;
    onerror: ((event: Event) => void) | null;
    send(data: BufferSource): void;
    close(code?: number, reason?: string): void;
  }

  export interface RfbOptions {
    shared?: boolean;
    credentials?: { username?: string; password?: string; target?: string };
    repeaterID?: string;
    wsProtocols?: readonly string[];
  }

  export interface RfbEventMap {
    connect: CustomEvent<Record<string, never>>;
    disconnect: CustomEvent<{ clean: boolean }>;
    clipboard: CustomEvent<{ text: string }>;
    securityfailure: CustomEvent<{ status: number; reason?: string }>;
    credentialsrequired: CustomEvent<{ types: string[] }>;
    desktopname: CustomEvent<{ name: string }>;
  }

  export default class RFB extends EventTarget {
    constructor(target: HTMLElement, urlOrChannel: string | RfbChannel, options?: RfbOptions);
    viewOnly: boolean;
    focusOnClick: boolean;
    clipViewport: boolean;
    dragViewport: boolean;
    scaleViewport: boolean;
    resizeSession: boolean;
    showDotCursor: boolean;
    background: string;
    qualityLevel: number;
    compressionLevel: number;
    disconnect(): void;
    focus(options?: FocusOptions): void;
    blur(): void;
    clipboardPasteFrom(text: string): void;
    /** Press and release when `down` is omitted. */
    sendKey(keysym: number, code: string | null, down?: boolean): void;
    addEventListener<K extends keyof RfbEventMap>(
      type: K,
      listener: (event: RfbEventMap[K]) => void,
      options?: AddEventListenerOptions | boolean,
    ): void;
    addEventListener(
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: AddEventListenerOptions | boolean,
    ): void;
    removeEventListener<K extends keyof RfbEventMap>(
      type: K,
      listener: (event: RfbEventMap[K]) => void,
      options?: EventListenerOptions | boolean,
    ): void;
    removeEventListener(
      type: string,
      listener: EventListenerOrEventListenerObject | null,
      options?: EventListenerOptions | boolean,
    ): void;
  }
}
