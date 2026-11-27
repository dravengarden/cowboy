export async function requestPurgeDrafts(
  request: (path: string, init?: RequestInit) => Promise<Response>,
  ids: readonly string[],
): Promise<string[]> {
  const unique = [...new Set(ids)];
  if (unique.length === 0) return [];
  const purged: string[] = [];
  for (let i = 0; i < unique.length; i += 500) {
    const batch = unique.slice(i, i + 500);
    const response = await request("/api/drafts/purge", {
      method: "POST",
      body: JSON.stringify({ ids: batch }),
    });
    const body = await response.json().catch(() => null) as { error?: string; purged?: unknown } | null;
    if (!response.ok) {
      throw new Error(body?.error ?? "Could not delete drafts permanently");
    }
    if (Array.isArray(body?.purged)) {
      for (const id of body.purged) {
        if (typeof id === "string") purged.push(id);
      }
    }
  }
  return purged;
}

export function purgedAnnouncementId(entry: unknown): string | null {
  if (!entry || typeof entry !== "object") return null;
  const candidate = entry as { id?: unknown; purged?: unknown };
  return candidate.purged === true && typeof candidate.id === "string" ? candidate.id : null;
}
