import { moduleEntryFromHtml } from "./serviceWorkerUpdates";

const MODULE_LOAD_PATTERNS =
  /(?:importing a module script failed|failed to fetch dynamically imported module|error loading dynamically imported module|chunkloaderror|loading chunk \S+ failed)/i;

export function isModuleLoadError(error: Error): boolean {
  return MODULE_LOAD_PATTERNS.test(`${error.name} ${error.message}`);
}

export function forcedBundleRecoveryUrl(
  currentUrl: string,
  now: () => number = Date.now,
): string {
  const target = new URL(currentUrl);
  target.searchParams.set("cowboy-recover", String(now()));
  return target.toString();
}

export async function latestBundleRecoveryUrl(
  currentUrl: string,
  origin: string,
  fetchRecovery: typeof fetch,
  now: () => number = Date.now,
): Promise<string | undefined> {
  const token = String(now());
  try {
    const indexUrl = new URL("/", origin);
    indexUrl.searchParams.set("cowboy-recover-probe", token);
    const index = await fetchRecovery(indexUrl, {
      cache: "no-store",
      headers: { "x-cowboy-recovery": "bundle-probe" },
    });
    if (!index.ok) return undefined;

    const entryPath = moduleEntryFromHtml(await index.text());
    if (!entryPath) return undefined;
    const entryUrl = new URL(entryPath, origin);
    if (entryUrl.origin !== origin) return undefined;
    const entry = await fetchRecovery(entryUrl, {
      cache: "no-store",
      headers: { "x-cowboy-recovery": "entry-probe" },
    });
    if (
      !entry.ok ||
      !/(?:javascript|ecmascript)/i.test(entry.headers.get("content-type") ?? "")
    ) return undefined;

    return forcedBundleRecoveryUrl(currentUrl, () => Number(token));
  } catch {
    return undefined;
  }
}
