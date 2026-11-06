export interface CheckRequest {
  operation: string;
  epoch: string;
}

/** Only an explicit refusal proves a request safe to forget. */
export async function sendCheckRequest(
  session: string,
  check: string,
  request: CheckRequest,
  fetcher: typeof fetch = fetch,
): Promise<{ forget: boolean; message?: string }> {
  try {
    const response = await fetcher(`/api/code/sessions/${encodeURIComponent(session)}/checks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ check, ...request }),
    });
    const body = (await response.json()) as { status?: string; reason?: string };
    if (response.ok && (body.status === "accepted" || body.status === "existing"))
      return { forget: true };
    if (body.status === "refused" || body.status === "epoch_changed") {
      return { forget: true, message: body.reason ?? "Check was not started." };
    }
  } catch {
    /* A missing or malformed reply is not proof of refusal. */
  }
  return { forget: false, message: "Check outcome is unknown. Retry to query the same request." };
}
