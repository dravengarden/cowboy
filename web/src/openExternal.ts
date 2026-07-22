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
};

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

const EXTERNAL_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

export function safeExternalUrl(url: string): string | null {
  try {
    const parsed = new URL(url);
    return EXTERNAL_PROTOCOLS.has(parsed.protocol) ? parsed.href : null;
  } catch {
    return null;
  }
}

function openInBrowser(url: string): void {
  window.open(url, "_blank", "noopener,noreferrer");
}
