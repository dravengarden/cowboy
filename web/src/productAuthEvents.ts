export const PRODUCT_AUTH_SESSION_EVENT = "cowboy:product-auth-session";
export const PRODUCT_AUTH_COOKIE_CHANGED_EVENT =
  "cowboy:product-auth-cookie-changed";

export function announceProductAuthSession(session: unknown): void {
  globalThis.dispatchEvent(
    new CustomEvent(PRODUCT_AUTH_SESSION_EVENT, {
      detail: session,
    }),
  );
}

export function announceProductAuthCookieChanged(): void {
  globalThis.dispatchEvent(new Event(PRODUCT_AUTH_COOKIE_CHANGED_EVENT));
}
