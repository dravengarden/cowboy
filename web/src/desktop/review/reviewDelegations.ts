// What Desktop Review asked a fork of the Agent to do (docs/desktop-code-review.md
// "The agent loop"): per device, per source Session. A review delegation
// remembers its task id and each file's change fingerprint when it was asked,
// so a finding can be drawn only while its file is unchanged since then.

import { useSyncExternalStore } from "react";

export interface DelegatedFile {
  readonly path: string;
  /** The change fingerprint when the review was asked; absent from older Machines. */
  readonly fingerprint?: string | undefined;
}

export interface ReviewDelegation {
  readonly fork: string;
  readonly task: string;
  readonly kind: "explain" | "review";
  readonly files: readonly DelegatedFile[];
  readonly atMs: number;
  /** The code context the review read (its files' current blobs come from
   *  there); absent on delegations recorded before it. */
  readonly context?: string | undefined;
}

const STORAGE_KEY = "cowboy.desktop.reviewDelegations.v1";
export const DELEGATION_LIMIT = 20;
const SOURCE_LIMIT = 50;

type Store = Readonly<Record<string, readonly ReviewDelegation[]>>;

function valid(value: unknown): value is ReviewDelegation {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return typeof record.fork === "string" && typeof record.task === "string" &&
    (record.kind === "explain" || record.kind === "review") && typeof record.atMs === "number" &&
    Array.isArray(record.files) && record.files.every((file) =>
      !!file && typeof (file as { path?: unknown }).path === "string"
    );
}

export function parseDelegations(raw: unknown): Store {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw as Record<string, unknown>).flatMap(([source, list]) =>
      Array.isArray(list) ? [[source, list.filter(valid).slice(-DELEGATION_LIMIT)]] : []
    ),
  );
}

/** `store` with `delegation` recorded for `source`, newest last and bounded. */
export function withDelegation(store: Store, source: string, delegation: ReviewDelegation): Store {
  const entries = Object.entries(store).filter(([id]) => id !== source);
  const list = [...(store[source] ?? []).filter((item) => item.task !== delegation.task), delegation];
  entries.push([source, list.slice(-DELEGATION_LIMIT)]);
  return Object.fromEntries(entries.slice(-SOURCE_LIMIT));
}

let cache: Store | undefined;
const listeners = new Set<() => void>();

function current(): Store {
  if (cache) return cache;
  try {
    cache = parseDelegations(JSON.parse(globalThis.localStorage?.getItem(STORAGE_KEY) ?? "{}"));
  } catch {
    cache = {};
  }
  return cache;
}

export function recordDelegation(source: string, delegation: ReviewDelegation): void {
  cache = withDelegation(current(), source, delegation);
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, JSON.stringify(cache));
  } catch {
    // The delegation then lasts for this page only.
  }
  for (const listener of listeners) listener();
}

const NONE: readonly ReviewDelegation[] = [];

export function useDelegations(source: string | undefined): readonly ReviewDelegation[] {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => source ? current()[source] ?? NONE : NONE,
    () => NONE,
  );
}

/** A task id `review.start` accepts: letters, digits, `-`, `_`, `.`, ≤128. */
export function delegationTask(nowMs: number, random: number): string {
  return `desktop-review-${nowMs.toString(36)}-${Math.floor(random * 36 ** 6).toString(36).padStart(6, "0")}`;
}
