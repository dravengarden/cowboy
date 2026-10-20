/** References only. Product authentication and the server's owner/account checks
 * remain authoritative; opening this route can never start a native login. */
export function subscriptionSignInLink(hash: string): { account: string; request: string } | undefined {
  const match = /^#subscription-sign-in\/([a-f0-9-]{36})\/(provider-login-[a-f0-9-]{36})$/.exec(hash);
  return match ? { account: match[1]!, request: match[2]! } : undefined;
}

export const subscriptionSignInLocation = () => subscriptionSignInLink(location.hash) ? location.hash : "";
export function subscribeSignInLocation(changed: () => void): () => void {
  window.addEventListener("hashchange", changed);
  return () => window.removeEventListener("hashchange", changed);
}

export function closeSubscriptionSignIn(): void {
  history.replaceState(history.state, "", location.pathname + location.search);
  window.dispatchEvent(new Event("hashchange"));
}
