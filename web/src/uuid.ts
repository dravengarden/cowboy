/**
 * Returns a UUID without assuming the page is a secure context.
 *
 * iOS WebViews and HTTP development origins can expose `crypto` while omitting
 * `randomUUID`. Keep identifiers globally unique when getRandomValues exists,
 * and retain a last-resort fallback for locked-down embedded browsers.
 */
export function newUuid(): string {
  const cryptoApi = globalThis.crypto;
  if (typeof cryptoApi?.randomUUID === "function") {
    try {
      return cryptoApi.randomUUID();
    } catch {
      // Fall through to getRandomValues/Math.random when the WebView exposes a
      // non-callable or policy-disabled randomUUID implementation.
    }
  }
  if (typeof cryptoApi?.getRandomValues === "function") {
    try {
      const bytes = cryptoApi.getRandomValues(new Uint8Array(16));
      bytes[6] = (bytes[6]! & 0x0f) | 0x40;
      bytes[8] = (bytes[8]! & 0x3f) | 0x80;
      const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    } catch {
      // Some private/embedded contexts deny both random APIs.
    }
  }
  return `fallback-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}
