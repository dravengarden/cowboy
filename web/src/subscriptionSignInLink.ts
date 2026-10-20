/** References only. Product authentication and the server's owner/account checks
 * remain authoritative; opening this route can never start a native login. */
export function subscriptionSignInLink(hash: string): { account: string; request: string } | undefined {
  const match = /^#subscription-sign-in\/([a-f0-9-]{36})\/(provider-login-[a-f0-9-]{36})$/.exec(hash);
  return match ? { account: match[1]!, request: match[2]! } : undefined;
}

export const SUBSCRIPTION_SIGN_IN_RETURN = "cowboy:subscription-sign-in-return";

/** Retain only the validated reference across a full-page product sign-in. */
export function rememberSubscriptionSignIn(hash = location.hash, storage?: Pick<Storage, "setItem">): void {
  if (subscriptionSignInLink(hash)) (storage ?? sessionStorage).setItem(SUBSCRIPTION_SIGN_IN_RETURN, hash);
}

/** Consume once on document boot. This restores navigation, never authority;
 * the handoff still mounts behind ProductAuthGate and verifies its owner. */
export function restoreSubscriptionSignIn(
  current: Pick<Location, "pathname" | "search" | "hash"> = location,
  navigation: Pick<History, "state" | "replaceState"> = history,
  storage?: Pick<Storage, "getItem" | "removeItem">,
): void {
  try {
    const saved = (storage ?? sessionStorage).getItem(SUBSCRIPTION_SIGN_IN_RETURN);
    (storage ?? sessionStorage).removeItem(SUBSCRIPTION_SIGN_IN_RETURN);
    if (current.pathname === "/" && !current.hash && saved && subscriptionSignInLink(saved)) {
      navigation.replaceState(navigation.state, "", current.pathname + current.search + saved);
    }
  } catch { /* A disabled browser store cannot prevent the ordinary login page. */ }
}

export const subscriptionSignInLocation = () => subscriptionSignInLink(location.hash) ? location.hash : "";
export function subscribeSignInLocation(changed: () => void): () => void {
  window.addEventListener("hashchange", changed);
  return () => window.removeEventListener("hashchange", changed);
}

export function closeSubscriptionSignIn(): void {
  try { sessionStorage.removeItem(SUBSCRIPTION_SIGN_IN_RETURN); } catch { /* Reference only. */ }
  history.replaceState(history.state, "", location.pathname + location.search);
  window.dispatchEvent(new Event("hashchange"));
}
