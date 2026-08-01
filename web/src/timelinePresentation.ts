import { linkTimeline } from "./derive";
import type { Envelope } from "./protocol";

export interface TimelinePresentationStep {
  timeline: Envelope[];
  complete: boolean;
}

/**
 * Reveal a history-only prepend while live presentation is frozen for native
 * scrolling.
 *
 * A column-reverse transcript inserts older rows above the viewport, so this
 * mutation cannot move the reader's current content. Live tail replacements or
 * appends are deliberately left frozen: those grow below the viewport and can
 * fight scroll anchoring. Existing envelopes stay reference-identical so their
 * Markdown and tool rows do not re-render.
 */
export function revealHistoryPrepend(
  current: Envelope[],
  latest: Envelope[],
): Envelope[] {
  if (current.length === 0 || latest.length <= current.length) return current;
  const firstSeq = current[0]?.seq;
  if (firstSeq === undefined) return current;
  const currentStart = latest.findIndex((event) => event.seq === firstSeq);
  if (currentStart <= 0 || currentStart + current.length > latest.length) {
    return current;
  }
  for (let index = 0; index < current.length; index += 1) {
    if (latest[currentStart + index]?.seq !== current[index]?.seq) return current;
  }
  return linkTimeline(
    [...latest.slice(0, currentStart), ...current],
    current,
  );
}

function textChunk(event: Envelope | undefined): string | null {
  if (
    event?.kind !== "update" ||
    (
      event.update.sessionUpdate !== "agent_message_chunk" &&
      event.update.sessionUpdate !== "agent_thought_chunk" &&
      event.update.sessionUpdate !== "user_message_chunk"
    ) ||
    event.update.content?.type !== "text"
  ) {
    return null;
  }
  return event.update.content.text ?? "";
}

function withText(event: Envelope, text: string): Envelope {
  if (event.kind !== "update" || event.update.content?.type !== "text") {
    return event;
  }
  return {
    ...event,
    update: {
      ...event.update,
      content: {
        ...event.update.content,
        text,
      },
    },
  };
}

/**
 * Advance a frozen transcript by a bounded amount of visible work.
 *
 * Live text chunks are coalesced into the timeline's last envelope, so merely
 * limiting the number of appended envelopes does not bound a catch-up commit:
 * one replacement envelope can contain the entire accumulated answer. Grow that
 * text by a character budget first, then append only a small number of ordinary
 * events. Non-append mutations (history prepend/retention/session replacement)
 * fall back to the canonical timeline because synthesising them would violate
 * ordering.
 */
export function advanceTimelinePresentation(
  current: Envelope[],
  latest: Envelope[],
  textBudget = 1_200,
  eventBudget = 2,
): TimelinePresentationStep {
  if (current === latest) return { timeline: latest, complete: true };
  if (latest.length === 0) return { timeline: latest, complete: true };
  if (current.length > latest.length) return { timeline: latest, complete: true };

  // Existing rows must remain the same append-only run. The tail is allowed to
  // be a coalesced text replacement with the same canonical sequence number.
  const stableCount = Math.max(0, current.length - 1);
  for (let index = 0; index < stableCount; index += 1) {
    if (current[index] !== latest[index]) {
      return { timeline: latest, complete: true };
    }
  }

  let next = current;
  if (current.length > 0) {
    const tailIndex = current.length - 1;
    const oldTail = current[tailIndex];
    const newTail = latest[tailIndex];
    if (!oldTail || !newTail || oldTail.seq !== newTail.seq) {
      return { timeline: latest, complete: true };
    }
    if (oldTail !== newTail) {
      const oldText = textChunk(oldTail);
      const newText = textChunk(newTail);
      if (
        oldText === null ||
        newText === null ||
        !newText.startsWith(oldText)
      ) {
        return { timeline: latest, complete: true };
      }
      const visibleText = newText.slice(
        0,
        Math.min(newText.length, oldText.length + textBudget),
      );
      const replacement = visibleText.length === newText.length
        ? newTail
        : withText(newTail, visibleText);
      next = linkTimeline(
        [...current.slice(0, tailIndex), replacement],
        current,
      );
      if (visibleText.length < newText.length) {
        return { timeline: next, complete: false };
      }
    }
  }

  let remainingEvents = eventBudget;
  while (next.length < latest.length && remainingEvents > 0) {
    const canonical = latest[next.length];
    if (!canonical) break;
    const canonicalText = textChunk(canonical);
    if (canonicalText !== null && canonicalText.length > textBudget) {
      next = linkTimeline(
        [...next, withText(canonical, canonicalText.slice(0, textBudget))],
        next,
      );
      return { timeline: next, complete: false };
    }
    next = linkTimeline([...next, canonical], next);
    remainingEvents -= 1;
  }

  return { timeline: next, complete: next.length === latest.length };
}
