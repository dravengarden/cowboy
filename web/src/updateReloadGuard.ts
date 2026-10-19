import type { AttemptStorage } from "./updateAttempt";

const KEY = "cowboy:client-update-navigation";

/** One tab's last attempted build transition. No timer can turn a failed
 *  transition back into permission to reload the same document forever. */
export function updateReloadFailed(
  storage: AttemptStorage | undefined,
  from: string,
  to: string,
): boolean {
  try {
    const attempt = JSON.parse(storage?.getItem(KEY) ?? "null");
    return attempt?.from === from && attempt?.to === to;
  } catch {
    return false;
  }
}

export function recordUpdateReload(
  storage: AttemptStorage | undefined,
  from: string,
  to: string,
): void {
  try {
    storage?.setItem(KEY, JSON.stringify({ from, to }));
  } catch {
    // Storage denial must not strand an explicitly requested update.
  }
}

/** Native WebKit may replay a document on reload(). Use a fresh navigation
 *  when there is no worker with a verified cached shell. */
export function clientUpdateNavigationUrl(current: string, now: number): string {
  const url = new URL(current);
  url.searchParams.set("cowboy-update", String(now));
  return url.toString();
}
