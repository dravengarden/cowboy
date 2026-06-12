// Am I running inside cowboy's NATIVE Tauri shell (vs the PWA / a browser)?
//
// The shell's native layer (`CowboyNativeTweaks.mm`) injects
// `window.__cowboyNativeShell = true` at document-start — but ONLY once it has
// installed native keyboard avoidance (it resizes the WKWebView so the layout
// viewport shrinks for the keyboard, the Obsidian/Capacitor model). That flag is
// the gate for dropping the PWA's keyboard architecture: in the shell the web can
// use normal flow (no `position: fixed`), no `translateZ(0)` repaint hack, and no
// composition transform dance — which are the ROOT of the iOS IME swallow / caret
// bugs (they exist only to work around WebKit's "no repaint inside position:fixed"
// on the PWA). See tasks/active/cowboy-native-keyboard-ime.
//
// Keyed on this DEDICATED flag (not merely `__TAURI__` / `__cowboyHaptic`, which a
// shell build injects regardless) so the gate stays inert until the native resize
// is actually present — shipping the web half early is then a safe no-op.
export function isNativeShell(): boolean {
  if (typeof window === "undefined") return false;
  return (window as { __cowboyNativeShell?: boolean }).__cowboyNativeShell === true;
}

// Read clipboard text, native-first. The iOS WKWebView shell does NOT grant
// `navigator.clipboard.readText()` (it rejects / returns empty — unlike Safari),
// which is why the in-composer Paste button silently no-op'd on the device. The
// native layer (`CowboyNativeTweaks.mm`) exposes a reply-style bridge at
// `window.__cowboyReadClipboard()` that reads `UIPasteboard.general` and returns a
// Promise<string>; prefer it in the shell. On the PWA / browser fall back to the
// web Clipboard API (where it works and shows the iOS paste affordance). Returns ""
// on any failure so callers can just guard on a non-empty result.
export async function readClipboardText(): Promise<string> {
  if (typeof window === "undefined") return "";
  const bridge = (window as { __cowboyReadClipboard?: () => Promise<unknown> }).__cowboyReadClipboard;
  if (typeof bridge === "function") {
    try {
      const t = await bridge();
      return typeof t === "string" ? t : "";
    } catch {
      return "";
    }
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}
