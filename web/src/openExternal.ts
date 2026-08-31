// Route rendered markdown links through the native Tauri opener when present;
// the browser/PWA path keeps normal window.open semantics. This adapter stays
// outside mdlive so the vendored engine remains framework-agnostic.
type TauriGlobal = {
  opener?: { openUrl?: (url: string) => Promise<void> };
  core?: { invoke?: (command: string, args: Record<string, unknown>) => Promise<unknown> };
};

type TauriInternals = {
  invoke?: (command: string, args: Record<string, unknown>) => Promise<unknown>;
};

type NativeGlobals = {
  __TAURI__?: TauriGlobal;
  __TAURI_INTERNALS__?: TauriInternals;
  __cowboyOpenAuthenticationBrowser?: (url: string) => boolean;
  __cowboyCloseAuthenticationBrowser?: () => void;
  __cowboyAuthenticationBrowserBridgeVersion?: number;
  __cowboyOpenPasskeyBrowser?: (url: string) => Promise<unknown>;
  __cowboyClosePasskeyBrowser?: () => void;
  __cowboyPasskeyBrowserBridgeVersion?: number;
};

export type NativePasskeyBrowserStatus =
  | "complete"
  | "cancelled"
  | "failed"
  | "unavailable";

export const NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT =
  "cowboy:native-authentication-browser-closed";
export const NATIVE_AUTHENTICATION_BROWSER_OPENED_EVENT =
  "cowboy:native-authentication-browser-opened";
export const NATIVE_AUTHENTICATION_BROWSER_OPEN_FAILED_EVENT =
  "cowboy:native-authentication-browser-open-failed";
export const NATIVE_APP_RESUMED_EVENT = "cowboy:native-resume";
const NATIVE_AUTHENTICATION_BROWSER_OPEN_TIMEOUT_MS = 5_000;

/** True only for a native shell that can hand an URL to the operating system.
 * Browser/PWA links must retain native anchor navigation instead of being
 * cancelled and recreated with window.open. */
export function hasNativeExternalOpener(): boolean {
  const root = globalThis as typeof globalThis & NativeGlobals;
  return typeof root.__TAURI__?.opener?.openUrl === "function" ||
    typeof root.__TAURI__?.core?.invoke === "function" ||
    typeof root.__TAURI_INTERNALS__?.invoke === "function";
}

export function shouldRouteExternalClick(event: {
  button: number;
  defaultPrevented: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): boolean {
  return hasNativeExternalOpener() && event.button === 0 &&
    !event.defaultPrevented && !event.altKey && !event.ctrlKey &&
    !event.metaKey && !event.shiftKey;
}

export function openExternalUrl(url: string): void {
  const resolved = safeExternalUrl(url);
  if (!resolved) return;
  const root = globalThis as typeof globalThis & NativeGlobals;
  const tauri = root.__TAURI__;

  if (tauri?.opener?.openUrl) {
    void tauri.opener.openUrl(resolved).catch(() => openInBrowser(resolved));
    return;
  }
  if (tauri?.core?.invoke) {
    void tauri.core.invoke("plugin:opener|open_url", { url: resolved })
      .catch(() => openInBrowser(resolved));
    return;
  }
  if (root.__TAURI_INTERNALS__?.invoke) {
    void root.__TAURI_INTERNALS__.invoke("plugin:opener|open_url", { url: resolved })
      .catch(() => openInBrowser(resolved));
    return;
  }
  openInBrowser(resolved);
}

/** Open an interactive Provider sign-in without replacing Cowboy's main view.
 * The iOS shell presents SFSafariViewController, Desktop uses the Tauri opener,
 * and browser/PWA callers open a new tab from the originating user gesture. */
export function openAuthenticationUrl(url: string): void {
  const resolved = safeAuthenticationUrl(url);
  if (!resolved) return;
  const root = globalThis as typeof globalThis & NativeGlobals;
  try {
    if (
      typeof root.__cowboyOpenAuthenticationBrowser === "function" &&
      root.__cowboyOpenAuthenticationBrowser(resolved) !== false
    ) return;
  } catch {
    // An older or partially initialized shell can still use the Tauri opener.
  }
  openExternalUrl(resolved);
}

/** Open an authentication sheet and, on bridge v2+, wait until UIKit confirms
 * that it is actually presented. Older shells keep their existing immediate
 * handoff behavior and receive the web lifecycle recovery path. */
export async function openAuthenticationUrlConfirmed(
  url: string,
  signal?: AbortSignal,
): Promise<void> {
  const resolved = safeAuthenticationUrl(url);
  if (!resolved) throw new Error("Authentication URL is invalid");
  const root = globalThis as typeof globalThis & NativeGlobals;
  if (
    typeof root.__cowboyOpenAuthenticationBrowser !== "function" ||
    (root.__cowboyAuthenticationBrowserBridgeVersion ?? 0) < 2
  ) {
    openAuthenticationUrl(resolved);
    return;
  }
  if (signal?.aborted) {
    throw signal.reason ?? new DOMException("Cancelled", "AbortError");
  }

  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const finish = (reason?: unknown): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      signal?.removeEventListener("abort", onAbort);
      globalThis.removeEventListener(
        NATIVE_AUTHENTICATION_BROWSER_OPENED_EVENT,
        onOpened,
      );
      globalThis.removeEventListener(
        NATIVE_AUTHENTICATION_BROWSER_OPEN_FAILED_EVENT,
        onFailed,
      );
      if (reason === undefined) resolve();
      else reject(reason);
    };
    const onOpened = (): void => finish();
    const onFailed = (): void =>
      finish(new Error("Cowboy could not open the authentication browser"));
    const onAbort = (): void =>
      finish(signal?.reason ?? new DOMException("Cancelled", "AbortError"));
    const timeout = setTimeout(
      () =>
        finish(new Error("Cowboy could not open the authentication browser")),
      NATIVE_AUTHENTICATION_BROWSER_OPEN_TIMEOUT_MS,
    );
    signal?.addEventListener("abort", onAbort, { once: true });
    globalThis.addEventListener(
      NATIVE_AUTHENTICATION_BROWSER_OPENED_EVENT,
      onOpened,
      { once: true },
    );
    globalThis.addEventListener(
      NATIVE_AUTHENTICATION_BROWSER_OPEN_FAILED_EVENT,
      onFailed,
      { once: true },
    );
    try {
      if (root.__cowboyOpenAuthenticationBrowser?.(resolved) === false) {
        finish();
        openExternalUrl(resolved);
      }
    } catch {
      finish();
      openExternalUrl(resolved);
    }
  });
}

export function closeAuthenticationBrowser(): void {
  const root = globalThis as typeof globalThis & NativeGlobals;
  try {
    root.__cowboyCloseAuthenticationBrowser?.();
  } catch {
    // Closing the Cowboy dialog must never depend on the optional native sheet.
  }
}

export function hasNativePasskeyAuthenticationBrowser(): boolean {
  const root = globalThis as typeof globalThis & NativeGlobals;
  return (root.__cowboyPasskeyBrowserBridgeVersion ?? 0) >= 1 &&
    typeof root.__cowboyOpenPasskeyBrowser === "function";
}

/** Run Cowboy's fixed-origin external Passkey page in an iOS web
 * authentication session. Its callback closes the system sheet even while the
 * underlying WKWebView is suspended. An unavailable or partial bridge falls
 * back to the existing Safari-sheet transport; cancellation never does. */
export async function openPasskeyAuthenticationUrl(
  url: string,
  signal?: AbortSignal,
): Promise<NativePasskeyBrowserStatus> {
  const resolved = safeAuthenticationUrl(url);
  if (!resolved) throw new Error("Passkey URL is invalid");
  const root = globalThis as typeof globalThis & NativeGlobals;
  const open = root.__cowboyOpenPasskeyBrowser;
  if (!hasNativePasskeyAuthenticationBrowser() || typeof open !== "function") {
    return "unavailable";
  }
  if (signal?.aborted) {
    throw signal.reason ?? new DOMException("Cancelled", "AbortError");
  }

  let request: Promise<unknown>;
  try {
    request = open(resolved);
  } catch {
    return "unavailable";
  }

  const raw = await new Promise<unknown>((resolve, reject) => {
    let settled = false;
    const finish = (result: { value: unknown } | { error: unknown }): void => {
      if (settled) return;
      settled = true;
      signal?.removeEventListener("abort", onAbort);
      if ("value" in result) resolve(result.value);
      else reject(result.error);
    };
    const onAbort = (): void => {
      try {
        root.__cowboyClosePasskeyBrowser?.();
      } catch {
        // The AbortSignal remains authoritative if the optional close fails.
      }
      finish({
        error: signal?.reason ?? new DOMException("Cancelled", "AbortError"),
      });
    };
    signal?.addEventListener("abort", onAbort, { once: true });
    void Promise.resolve(request).then(
      (value) => finish({ value }),
      () => finish({ value: { ok: false } }),
    );
  });

  if (raw == null || typeof raw !== "object") return "unavailable";
  const reply = raw as { ok?: unknown; status?: unknown };
  if (reply.ok !== true) return "unavailable";
  if (
    reply.status === "complete" || reply.status === "cancelled" ||
    reply.status === "failed"
  ) {
    return reply.status;
  }
  return "unavailable";
}

export function hasNativeAuthenticationBrowser(): boolean {
  const root = globalThis as typeof globalThis & NativeGlobals;
  return typeof root.__cowboyOpenAuthenticationBrowser === "function";
}

export function shouldRouteAuthenticationClick(event: {
  button: number;
  defaultPrevented: boolean;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
}): boolean {
  return (hasNativeAuthenticationBrowser() || hasNativeExternalOpener()) &&
    event.button === 0 && !event.defaultPrevented && !event.altKey &&
    !event.ctrlKey && !event.metaKey && !event.shiftKey;
}

const EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export function safeExternalUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    return EXTERNAL_PROTOCOLS.has(parsed.protocol) ? parsed.href : null;
  } catch {
    return null;
  }
}

export function safeAuthenticationUrl(url: string): string | null {
  const resolved = safeExternalUrl(url);
  if (!resolved) return null;
  const protocol = new URL(resolved).protocol;
  return protocol === "https:" || protocol === "http:" ? resolved : null;
}

function openInBrowser(url: string): void {
  window.open(url, "_blank", "noopener,noreferrer");
}
