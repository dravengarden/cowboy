import { invokeRegisteredMessageCopy, type MessageCopyMode } from "../messageCopyRegistry";
import { notify } from "../store";

export interface MessageRectCandidate {
  readonly copyId: string;
  readonly role: "assistant" | "user";
  readonly top: number;
  readonly bottom: number;
}

export function rectIntersectsViewport(
  top: number,
  bottom: number,
  viewTop: number,
  viewBottom: number,
  margin = 8,
): boolean {
  return bottom > viewTop + margin && top < viewBottom - margin;
}

/** Pick the message the reader is looking at for Conversation Vim `y`/`Y`:
 *  1. If a message spans the viewport reading focal line (45% from top), pick
 *     it immediately — tall replies always win while reading them.
 *  2. Otherwise prefer the visible assistant reply closest to the focal line,
 *     falling back to any visible user bubble when only user messages show. */
export function pickViewportMessageCopyId(
  candidates: readonly MessageRectCandidate[],
  viewTop: number,
  viewBottom: number,
): string | null {
  const visible = candidates.filter((c) => rectIntersectsViewport(c.top, c.bottom, viewTop, viewBottom));
  if (visible.length === 0) return null;
  const focalY = viewTop + (viewBottom - viewTop) * 0.45;
  const spanning = visible.find((c) => c.top <= focalY && c.bottom >= focalY);
  if (spanning) return spanning.copyId;
  const assistants = visible.filter((c) => c.role === "assistant");
  const pool = assistants.length > 0 ? assistants : visible;
  let best = pool[0]!;
  let bestDist = Math.abs((best.top + best.bottom) / 2 - focalY);
  for (let i = 1; i < pool.length; i++) {
    const candidate = pool[i]!;
    const dist = Math.abs((candidate.top + candidate.bottom) / 2 - focalY);
    if (dist < bestDist) {
      best = candidate;
      bestDist = dist;
    }
  }
  return best.copyId;
}

function resolveTranscriptCopyId(region: HTMLElement): string | null {
  const scroller = region.querySelector<HTMLElement>("[data-desktop-transcript-scroller]") ?? region;
  const viewRect = scroller.getBoundingClientRect();
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const focusedWrap = active?.closest<HTMLElement>("[data-message-copy-id]") ?? null;
  if (focusedWrap && region.contains(focusedWrap) && focusedWrap.dataset.messageCopyId) {
    const focusedRect = focusedWrap.getBoundingClientRect();
    if (
      focusedRect.height > 0 &&
      focusedRect.width > 0 &&
      rectIntersectsViewport(focusedRect.top, focusedRect.bottom, viewRect.top, viewRect.bottom)
    ) {
      return focusedWrap.dataset.messageCopyId;
    }
  }
  const wraps = [...region.querySelectorAll<HTMLElement>("[data-message-copy-id]")];
  const candidates: MessageRectCandidate[] = [];
  for (const el of wraps) {
    const copyId = el.dataset.messageCopyId;
    if (!copyId) continue;
    const rect = el.getBoundingClientRect();
    if (rect.height <= 0 || rect.width <= 0) continue;
    const role = el.dataset.messageCopyRole === "user" ? "user" : "assistant";
    candidates.push({ copyId, role, top: rect.top, bottom: rect.bottom });
  }
  return pickViewportMessageCopyId(candidates, viewRect.top, viewRect.bottom);
}

/** Handle Conversation Normal-mode `y` (Copy) and `Y` (Copy as Markdown). */
export function tryHandleTranscriptYank(
  event: KeyboardEvent,
  region: HTMLElement | null,
  key: string,
  activeOtherControl: boolean,
): boolean {
  if (event.ctrlKey || activeOtherControl || !region) return false;
  if (key !== "y" && key !== "Y") return false;
  const copyId = resolveTranscriptCopyId(region);
  if (!copyId) return false;
  event.preventDefault();
  event.stopPropagation();
  if (event.repeat) return true;
  const mode: MessageCopyMode = key === "Y" ? "markdown" : "rich";
  void invokeRegisteredMessageCopy(copyId, mode).then((ok) => {
    if (ok) {
      notify(mode === "markdown" ? "Copied message as Markdown" : "Copied message", "info");
    }
  });
  return true;
}
