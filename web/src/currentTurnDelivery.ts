export type CurrentTurnOutcome = "injected" | "not_delivered" | "unknown";

/** Only the originating gesture may POST. Reload/reconnect and explicit Retry
 * observe the original receipt; they cannot inject into a later turn. */
export async function deliverCurrentTurn(
  session: string,
  operation: string,
  payload: { text: string; content: unknown[] } | null,
  request: typeof fetch = fetch,
): Promise<CurrentTurnOutcome> {
  const root = `/api/sessions/${encodeURIComponent(session)}`;
  const deadline = Date.now() + 35_000;
  try {
    let response = payload === null
      ? await request(`${root}/delivery/${encodeURIComponent(operation)}`, {
        signal: AbortSignal.timeout(5000),
      })
      : await request(`${root}/prompt`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          ...payload,
          delivery: "current_turn",
          operation,
        }),
        signal: AbortSignal.timeout(5000),
      });
    // Absence after a restored outbox means no delivery is confirmed. Do not
    // POST it now: that would target a different turn after a reload.
    if (payload === null && response.status === 404) return "not_delivered";
    for (let attempt = 0; attempt < 32 && Date.now() < deadline; attempt++) {
      if (!response.ok) return "unknown";
      const receipt = await response.json() as { status?: string };
      if (receipt.status === "injected" || receipt.status === "not_delivered") {
        return receipt.status;
      }
      if (receipt.status !== "pending") {
        return "unknown";
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
      response = await request(
        `${root}/delivery/${encodeURIComponent(operation)}`,
        { signal: AbortSignal.timeout(5000) },
      );
    }
  } catch {
    // A lost POST response is never evidence that the native effect failed.
  }
  return "unknown";
}
