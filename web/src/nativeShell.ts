// Did the native Tauri layer install WKWebView keyboard resizing?
//
// `CowboyNativeTweaks.mm` injects this dedicated flag at document-start only
// after installing native keyboard avoidance. The editor itself now has one
// normal-flow implementation; the remaining consumer uses the flag to avoid
// applying a browser visualViewport inset on top of the native resize.
//
// Do not substitute a generic `__TAURI__` or haptics flag: a shell can provide
// those without providing the keyboard contract. See
// work-items/archive/2026/07/cowboy-native-keyboard-ime.
export function isNativeShell(): boolean {
  if (typeof window === "undefined") return false;
  return (window as { __cowboyNativeShell?: boolean }).__cowboyNativeShell === true;
}
