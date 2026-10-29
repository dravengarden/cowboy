export type CurrentTurnOutcome =
  | "injected"
  | "submitted"
  | "not_delivered"
  | "unknown";

export interface DeliveryTarget {
  worker_epoch: string;
  turn_id: string;
}

export async function captureDeliveryTarget(
  session: string,
): Promise<DeliveryTarget> {
  // Captured at activation when the picker had no fresh target yet; retried
  // like the picker's read so one slow request does not refuse the Steer.
  let response: Response | null = null;
  let failure: unknown = null;
  for (const delay of [0, 400, 1200]) {
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
    try {
      response = await fetch(
        `/api/sessions/${encodeURIComponent(session)}/delivery-options`,
        {
          signal: AbortSignal.timeout(5000),
          cache: "no-store",
        },
      );
      if (response.ok) break;
    } catch (error) {
      failure = error;
      response = null;
    }
  }
  if (!response?.ok) {
    throw failure instanceof Error && failure.name !== "TimeoutError"
      ? failure
      : new Error("Delivery options are unavailable.");
  }
  const options = await response.json() as { target?: DeliveryTarget | null };
  if (!options.target) {
    throw new Error("The active turn is no longer available.");
  }
  return options.target;
}

/** Only the originating gesture may POST. Reload/reconnect and explicit Retry
 * observe the original receipt; they cannot inject into a later turn. */
export async function deliverCurrentTurn(
  session: string,
  operation: string,
  payload: {
    text: string;
    content: unknown[];
    target?: DeliveryTarget;
    nextCheckpoint?: boolean;
  } | null,
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
          text: payload.text,
          content: payload.content,
          target: payload.target,
          delivery: payload.nextCheckpoint
            ? "next_checkpoint"
            : "current_turn",
          operation,
        }),
        signal: AbortSignal.timeout(5000),
      });
    // Absence after a restored outbox means no delivery is confirmed. Do not
    // POST it now: that would target a different turn after a reload.
    if (payload === null && response.status === 404) return "unknown";
    for (let attempt = 0; attempt < 32 && Date.now() < deadline; attempt++) {
      if (!response.ok) return "unknown";
      const receipt = await response.json() as { status?: string };
      if (
        receipt.status === "injected" || receipt.status === "submitted" ||
        receipt.status === "not_delivered"
      ) {
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
