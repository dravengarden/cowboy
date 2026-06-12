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
