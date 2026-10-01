/** Cached paint is not server acknowledgement. Keep recovering the focused tail. */
export function transcriptNeedsHydration(
  hydrated: boolean,
  source: "replica" | "live" | undefined,
): boolean {
  return !hydrated || source === "replica";
}

export function transcriptRetryDelay(attempt: number): number {
  return [750, 2_000, 10_000][attempt] ?? 30_000;
}
