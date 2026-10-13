import {
  extensionRequest,
  type PullSummary,
  type RepositoryRemote,
  resourceQuery,
} from "../../extensions/api";
import { type Choice, choices } from "./remoteReviewSources";

/** The repository's open pull requests, as far as one installed extension sees. */
export interface RepositoryPulls {
  readonly choice: Choice;
  readonly remotes: readonly RepositoryRemote[];
  readonly pulls: readonly PullSummary[];
  /** More open pull requests exist than were read. */
  readonly incomplete: boolean;
}

const PAGES = 3;
const FRESH_MS = 120_000;
const cache = new Map<string, { at: number; value: RepositoryPulls | null }>();

async function read(
  context: string,
  signal: AbortSignal,
): Promise<RepositoryPulls | null> {
  const inventory = await extensionRequest(context, null, signal);
  if (inventory.type !== "inventory") return null;
  const capable = choices(inventory).filter((choice) =>
    choice.extension.views.some((view) =>
      view.id === choice.view && view.discovery
    )
  );
  // Git's conventional publishing remote first; otherwise whatever exists.
  const choice = capable.find((item) => item.remote.name === "origin") ??
    capable[0];
  if (!choice) return null;
  const repository = `${choice.remote.owner}/${choice.remote.repository}`
    .toLowerCase();
  const pulls: PullSummary[] = [];
  let account: string | undefined;
  let incomplete = false;
  for (let page = 1; page <= PAGES; page++) {
    const query = resourceQuery(
      choice.extension.identity,
      choice.remote.name,
      choice.view,
      "",
      page,
    );
    query.set("discovery", "true");
    query.set("relation", "branches");
    query.set("state", "open");
    query.set("currentRepository", "true");
    if (account) query.set("account", account);
    const response = await extensionRequest(context, query, signal);
    if (response.type !== "pulls") return null;
    if (account && response.account !== account) return null;
    account = response.account;
    pulls.push(
      ...response.items.filter((item) =>
        item.repository.toLowerCase() === repository
      ),
    );
    incomplete = response.incomplete ||
      (response.nextPage !== null && page === PAGES);
    if (response.nextPage === null) break;
  }
  return { choice, remotes: inventory.remotes, pulls, incomplete };
}

/**
 * Read the open pull requests of a code context's repository. Resolves `null`
 * when there is nothing to show: no extension offers pull requests, its
 * Machine predates the listing, or the connection is unavailable. Pull
 * requests decorate the repository; their absence is never an error state.
 */
export async function loadRepositoryPulls(
  context: string,
  signal: AbortSignal,
  now: number = Date.now(),
): Promise<RepositoryPulls | null> {
  const cached = cache.get(context);
  if (cached && now - cached.at < FRESH_MS) return cached.value;
  try {
    const value = await read(context, signal);
    if (!signal.aborted) cache.set(context, { at: now, value });
    return value;
  } catch (reason) {
    if (signal.aborted) throw reason;
    // Remember the miss as well, or every drawer open repeats a failing read.
    cache.set(context, { at: now, value: null });
    return null;
  }
}

export function forgetRepositoryPulls(context: string): void {
  cache.delete(context);
}
