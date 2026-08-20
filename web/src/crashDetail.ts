import type { Status } from "./protocol";
import type { RenderItem } from "./derive";

/** Pull the human sentence out of Codex-style `Internal error: {JSON}` dumps. */
export function prettifyCrashDetail(raw: string): string {
  const trimmed = raw.trim();
  const jsonStart = trimmed.indexOf("{");
  if (jsonStart >= 0) {
    try {
      const parsed = JSON.parse(trimmed.slice(jsonStart)) as unknown;
      if (parsed && typeof parsed === "object" && "message" in parsed) {
        const message = (parsed as { message: unknown }).message;
        if (typeof message === "string" && message.trim()) {
          return message.trim();
        }
      }
    } catch {
      // Keep the original diagnostic when the payload isn't JSON.
    }
  }
  return trimmed.replace(/^Internal error:\s*/i, "").trim() || raw;
}

export function crashDetailsMatch(
  left: string | null | undefined,
  right: string | null | undefined,
): boolean {
  if (left == null || right == null) return left == null && right == null;
  return prettifyCrashDetail(left) === prettifyCrashDetail(right);
}

/**
 * While SessionStatusBar is showing the live crash, drop matching trailing
 * lifecycle rows so the same JSON dump isn't painted twice.
 */
export function hideLiveCrashDuplicate(
  items: readonly RenderItem[],
  status: Status,
  crashDetail: string | null,
): RenderItem[] {
  if (status !== "crashed" || crashDetail == null) {
    return items as RenderItem[];
  }
  let end = items.length;
  while (end > 0) {
    const item = items[end - 1];
    if (item?.kind !== "lifecycle" || item.status !== "crashed") break;
    if (
      item.detail && crashDetail && !crashDetailsMatch(item.detail, crashDetail)
    ) {
      break;
    }
    end -= 1;
  }
  return end === items.length ? items as RenderItem[] : items.slice(0, end);
}
