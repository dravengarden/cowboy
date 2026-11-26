// Interactive WebSocket client identity helpers shared by store.ts.
import { isAppleTouchWebView } from "./connectionRecovery";
import { newUuid } from "./uuid";

export function stableInteractiveClientId(): string {
  const key = "cowboy:interactive-client-id";
  try {
    const existing = globalThis.localStorage.getItem(key);
    if (existing !== null && existing.length >= 16 && existing.length <= 128) {
      return existing;
    }
    const created = newUuid();
    globalThis.localStorage.setItem(key, created);
    return created;
  } catch {
    // Storage may be unavailable in a private or transient WebView. The server
    // still namespaces this ephemeral id to the authenticated account.
    return `ephemeral-${newUuid()}`;
  }
}

export function interactiveClientKind(): "browser" | "native_shell" {
  const runtime = globalThis as typeof globalThis & { __TAURI_INTERNALS__?: unknown };
  const navigator = globalThis.navigator;
  return isAppleTouchWebView(
      navigator?.userAgent ?? "",
      navigator?.platform ?? "",
      navigator?.maxTouchPoints ?? 0,
    ) || runtime.__TAURI_INTERNALS__ !== undefined
    ? "native_shell"
    : "browser";
}
