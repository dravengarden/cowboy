function responseDetail(body: string): string | undefined {
  const text = body.trim();
  // A proxy's HTML error page is not a useful notification message.
  if (!text || text.startsWith("<")) return undefined;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return text;
  }
  if (typeof value === "string") return value.trim() || undefined;
  if (value === null || typeof value !== "object") return undefined;
  const record = value as Record<string, unknown>;
  for (const field of [record.detail, record.message, record.error]) {
    if (typeof field === "string" && field.trim()) return field.trim();
  }
  const error = record.error;
  if (error !== null && typeof error === "object" && "message" in error) {
    if (typeof error.message === "string") {
      return error.message.trim() || undefined;
    }
  }
  return undefined;
}

/** Preserve the operation, status and server explanation without consuming a
 * successful response. The same message works inline and in action feedback. */
export async function expectHttpOk(
  response: Response,
  action: string,
): Promise<void> {
  if (response.ok) return;
  let detail: string | undefined;
  try {
    detail = responseDetail(await response.text());
  } catch (cause) {
    if (cause instanceof Error && cause.name === "AbortError") throw cause;
    // Keep the HTTP failure when the error body itself could not be read.
  }
  throw new Error(
    `${action} (HTTP ${String(response.status)}): ${
      detail || response.statusText || "The server returned no error details."
    }`,
  );
}
