export type ReviewScrollElement = Pick<
  HTMLElement,
  "clientHeight" | "scrollHeight" | "scrollTop"
>;

function finiteNonNegative(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : undefined;
}

export function safeReviewScrollTop(
  value: unknown,
  scrollHeight: unknown,
  clientHeight: unknown,
): number {
  const requested = finiteNonNegative(value);
  const height = finiteNonNegative(scrollHeight);
  const viewport = finiteNonNegative(clientHeight);
  if (
    requested === undefined || height === undefined || viewport === undefined
  ) {
    return 0;
  }
  return Math.min(requested, Math.max(0, height - viewport));
}

export function restoreReviewScrollTop(
  element: ReviewScrollElement,
  value: unknown,
): number {
  try {
    const top = safeReviewScrollTop(
      value,
      element.scrollHeight,
      element.clientHeight,
    );
    element.scrollTop = top;
    return top;
  } catch {
    try {
      element.scrollTop = 0;
    } catch {
      // Scroll restoration is a convenience. A detached or hostile element
      // must never break tab activation.
    }
    return 0;
  }
}
