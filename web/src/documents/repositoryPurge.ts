export interface PurgeDraftTarget {
  readonly id: string;
  readonly expected_revision: number;
  readonly expected_trashed_at_ms: number;
}

export interface PurgedDraftReceipt {
  readonly id: string;
  readonly revision: number;
}

export async function requestPurgeDrafts(
  request: (path: string, init?: RequestInit) => Promise<Response>,
  targets: readonly PurgeDraftTarget[],
): Promise<PurgedDraftReceipt[]> {
  const unique = new Map<string, PurgeDraftTarget>();
  for (const target of targets) unique.set(target.id, target);
  const list = [...unique.values()];
  if (list.length === 0) return [];
  const purged: PurgedDraftReceipt[] = [];
  for (let i = 0; i < list.length; i += 500) {
    const batch = list.slice(i, i + 500);
    const response = await request("/api/drafts/purge", {
      method: "POST",
      body: JSON.stringify({ targets: batch }),
    });
    const body = await response.json().catch(() => null) as {
      error?: string;
      purged?: unknown;
      entries?: unknown;
    } | null;
    if (!response.ok) {
      throw new Error(body?.error ?? "Could not delete drafts permanently");
    }
    if (Array.isArray(body?.entries)) {
      for (const entry of body.entries) {
        if (
          entry && typeof entry === "object" &&
          typeof (entry as { id?: unknown }).id === "string" &&
          typeof (entry as { revision?: unknown }).revision === "number"
        ) {
          purged.push({
            id: (entry as { id: string }).id,
            revision: (entry as { revision: number }).revision,
          });
        }
      }
    } else if (Array.isArray(body?.purged)) {
      for (const id of body.purged) {
        if (typeof id === "string") {
          const matched = unique.get(id);
          purged.push({ id, revision: (matched?.expected_revision ?? 0) + 1 });
        }
      }
    }
  }
  return purged;
}

export function purgedAnnouncement(entry: unknown): PurgedDraftReceipt | null {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as { id?: unknown; purged?: unknown; revision?: unknown };
  if (candidate.purged !== true || typeof candidate.id !== "string") return null;
  const revision = typeof candidate.revision === "number" ? candidate.revision : Number.MAX_SAFE_INTEGER;
  return { id: candidate.id, revision };
}

export function clearDocumentSnapshots(snapshots: Map<string, unknown>, id: string): void {
  const prefix = `${id}:`;
  for (const key of snapshots.keys()) {
    if (key.startsWith(prefix)) snapshots.delete(key);
  }
}

export function collectOfflinePurged(
  owners: ReadonlyMap<string, { isCleanServerDoc(): boolean }>,
  serverIds: ReadonlySet<string>,
  served: ReadonlyMap<string, number>,
): PurgedDraftReceipt[] {
  const gone: PurgedDraftReceipt[] = [];
  for (const [docId, owner] of owners.entries()) {
    if (!serverIds.has(docId) && owner.isCleanServerDoc()) {
      gone.push({ id: docId, revision: (served.get(docId) ?? 0) + 1 });
    }
  }
  return gone;
}

export function sanitizeDraftTitle(raw: string): string {
  const cleaned = raw
    .replace(/\p{Cc}+/gu, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!cleaned) return "Draft";
  return [...cleaned].slice(0, 80).join("");
}
