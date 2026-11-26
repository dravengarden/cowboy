// FREEZE-WHILE-DETACHED anchor and column-reverse scroll helpers.
//
// Held in a ref so both the scroll listener (capture) and the per-chunk
// timeline layout effect (restore) share one anchor. `self` flags our own
// corrective scrollTop write so the listener swallows the scroll event it
// triggers.
export type FreezeAnchorAttr = "data-key" | "data-ancestor-item";

export interface FreezeAnchor {
  attr: FreezeAnchorAttr;
  key: string | null;
  top: number;
  self: boolean;
}

const ANCHOR_ROW_SELECTOR = "[data-key], [data-ancestor-item]";
const PROBE_OFFSETS = [0, -14, 14, -28, 28] as const;

export function createFreezeAnchor(): FreezeAnchor {
  return { attr: "data-key", key: null, top: 0, self: false };
}

/** Return true only when `nonce` increased for the SAME session. Because App
 *  reuses one `<Transcript>` instance across sessions, a session switch changes
 *  `sessionId` and must seed the ref without replaying the new session's older
 *  nonce against its restored viewport. */
export function shouldTriggerSessionNonce(
  previous: { sessionId: string; nonce: number },
  sessionId: string,
  nonce: number,
): boolean {
  if (previous.sessionId !== sessionId) {
    previous.sessionId = sessionId;
    previous.nonce = nonce;
    return false;
  }
  if (nonce === previous.nonce) return false;
  const increased = nonce > previous.nonce;
  previous.nonce = nonce;
  return increased;
}

/** `column-reverse` already keeps a followed transcript at scrollTop 0. Writing
 *  the same value for every streamed ACP envelope is not a harmless no-op on
 *  iOS WebKit: it schedules a scroll-layer update while adjacent thought and
 *  tool rows are still being laid out. Only correct real drift from the live
 *  edge; mark `anchor.self` when provided so our programmatic write is not
 *  mistaken for reader motion. */
export function pinTranscriptToLatest(
  el: HTMLElement,
  anchor?: FreezeAnchor,
): void {
  if (Math.abs(el.scrollTop) > 0.5) {
    if (anchor) anchor.self = true;
    el.scrollTop = 0;
  }
}

/** `column-reverse` uses opposite `scrollTop` signs across engines (negative
 *  in Blink/Gecko/modern WebKit, positive in older WebKit). Probe 1px away
 *  from 0 synchronously and restore `before` without mutating `anchor.self` so
 *  an immediately following keyboard/scroll write is never swallowed. */
export function columnReverseAwayDirection(el: HTMLElement): 1 | -1 {
  if (el.scrollTop > 0.5) return 1;
  if (el.scrollTop < -0.5) return -1;
  if (el.scrollHeight <= el.clientHeight + 1) return -1;
  const before = el.scrollTop;
  el.scrollBy({ top: -1, behavior: "auto" });
  if (el.scrollTop < -0.5) {
    el.scrollTop = before;
    return -1;
  }
  el.scrollBy({ top: 1, behavior: "auto" });
  const sign = el.scrollTop > 0.5 ? 1 : -1;
  el.scrollTop = before;
  return sign;
}

/** Compute the unobscured reading band inside `el` by insetting the container's
 *  top and bottom paddings (which carry `topInset` and `bottomInset`). */
export function unobscuredReadingBand(
  rect: Pick<DOMRect, "top" | "bottom" | "height">,
  paddingTop: number,
  paddingBottom: number,
): { top: number; bottom: number; midY: number } {
  const safeTop = rect.top + Math.max(0, paddingTop);
  const safeBottom = rect.bottom - Math.max(0, paddingBottom);
  if (safeBottom > safeTop + 8) {
    return { top: safeTop, bottom: safeBottom, midY: (safeTop + safeBottom) / 2 };
  }
  return { top: rect.top, bottom: rect.bottom, midY: rect.top + rect.height / 2 };
}

export function unobscuredReadingMidY(
  rect: Pick<DOMRect, "top" | "bottom" | "height">,
  paddingTop: number,
  paddingBottom: number,
): number {
  return unobscuredReadingBand(rect, paddingTop, paddingBottom).midY;
}

function readAnchorRow(
  el: HTMLElement,
  row: HTMLElement,
): { attr: FreezeAnchorAttr; key: string } | null {
  if (!el.contains(row)) return null;
  const key = row.getAttribute("data-key");
  if (key !== null) return { attr: "data-key", key };
  const ancestorKey = row.getAttribute("data-ancestor-item");
  if (ancestorKey !== null) return { attr: "data-ancestor-item", key: ancestorKey };
  return null;
}

/** Binary search a monotonically ordered list of row elements for the one
 *  covering or nearest `midY`. `descendingTop` is true for column-reverse
 *  `[data-key]` rows (DOM index 0 = visually lowest / largest `top`) and false
 *  for normal top-to-bottom `[data-ancestor-item]` rows. */
export function binarySearchRowNearY(
  rows: ArrayLike<HTMLElement>,
  elRect: Pick<DOMRect, "top" | "bottom">,
  midY: number,
  descendingTop: boolean,
): HTMLElement | null {
  const count = rows.length;
  if (count === 0) return null;
  let lo = 0;
  let hi = count - 1;
  let best: HTMLElement | null = null;
  let bestDist = Number.POSITIVE_INFINITY;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    const row = rows[mid];
    if (!row) break;
    const r = row.getBoundingClientRect();
    if (r.height > 0 && r.bottom > elRect.top && r.top < elRect.bottom) {
      if (r.top <= midY && r.bottom >= midY) return row;
      const dist = r.top > midY ? r.top - midY : midY - r.bottom;
      if (dist < bestDist) {
        bestDist = dist;
        best = row;
      }
    }
    if (r.top > midY) {
      // Row is visually below midY → move visually upward.
      if (descendingTop) lo = mid + 1;
      else hi = mid - 1;
    } else {
      // Row is visually above midY → move visually downward.
      if (descendingTop) hi = mid - 1;
      else lo = mid + 1;
    }
  }
  return best;
}

/** O(log n) fallback when all `elementFromPoint` probes miss (for example a
 *  floating overlay covering the reading band). Searches `[data-key]` rows in
 *  column-reverse order first, then `[data-ancestor-item]` rows if needed. */
export function pickVisibleAnchorRow(
  el: HTMLElement,
  elRect: Pick<DOMRect, "top" | "bottom">,
  midY: number,
): HTMLElement | null {
  const keyRows = el.querySelectorAll<HTMLElement>("[data-key]");
  const liveMatch = binarySearchRowNearY(keyRows, elRect, midY, true);
  if (liveMatch) {
    const r = liveMatch.getBoundingClientRect();
    if (r.top <= midY && r.bottom >= midY) return liveMatch;
  }
  const ancestorRows = el.querySelectorAll<HTMLElement>("[data-ancestor-item]");
  const ancMatch = binarySearchRowNearY(ancestorRows, elRect, midY, false);
  if (!liveMatch) return ancMatch;
  if (!ancMatch) return liveMatch;
  const lr = liveMatch.getBoundingClientRect();
  const ar = ancMatch.getBoundingClientRect();
  const lDist = lr.top > midY ? lr.top - midY : lr.bottom < midY ? midY - lr.bottom : 0;
  const aDist = ar.top > midY ? ar.top - midY : ar.bottom < midY ? midY - ar.bottom : 0;
  return aDist < lDist ? ancMatch : liveMatch;
}

/** Snapshot the visible row at the unobscured reading-band centre + its offset
 *  from the container top. Probes a few tight y-offsets in O(1) to bridge
 *  inter-row gaps, then falls back to an O(log n) binary search when overlays
 *  cover the probe points. */
export function captureFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  const r = el.getBoundingClientRect();
  const style = el.ownerDocument.defaultView?.getComputedStyle(el);
  const padTop = style ? Number.parseFloat(style.paddingTop) || 0 : 0;
  const padBottom = style ? Number.parseFloat(style.paddingBottom) || 0 : 0;
  const band = unobscuredReadingBand(r, padTop, padBottom);
  const cx = r.left + r.width / 2;

  let hitRow: HTMLElement | null = null;
  for (const offset of PROBE_OFFSETS) {
    const y = band.midY + offset;
    if (y < band.top || y > band.bottom) continue;
    const hit = el.ownerDocument.elementFromPoint(cx, y);
    const candidate = hit?.closest<HTMLElement>(ANCHOR_ROW_SELECTOR) ?? null;
    if (candidate && el.contains(candidate)) {
      hitRow = candidate;
      break;
    }
  }
  const row = hitRow ?? pickVisibleAnchorRow(el, r, band.midY);
  const resolved = row ? readAnchorRow(el, row) : null;
  if (!row || !resolved) {
    a.key = null;
    return;
  }
  a.attr = resolved.attr;
  a.key = resolved.key;
  a.top = row.getBoundingClientRect().top - r.top;
}

/** Re-assert the captured anchor's offset. `d(anchor.screenTop)/d(scrollTop) = -1`
 *  in any container, so `scrollTop += delta` pulls the anchor back to where it
 *  was — sign-agnostic, so it works for column-reverse's negative scrollTop too. */
export function restoreFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  if (a.key === null) return;
  const row = el.querySelector<HTMLElement>(
    `[${a.attr}="${CSS.escape(a.key)}"]`,
  );
  if (!row) return;
  const delta = row.getBoundingClientRect().top -
    el.getBoundingClientRect().top - a.top;
  if (Math.abs(delta) < 0.5) return;
  a.self = true;
  el.scrollTop += delta;
}

/** Once an older page replaces its placeholder boundary, move only a detached
 *  idle reader who is still sitting inside the boundary just past it. Only
 *  sets `anchor.self` and writes `scrollTop` when the target actually differs. */
export function applyScrollbackReplacement(
  el: HTMLElement,
  anchor: FreezeAnchor,
  computeReplacementFromTop: (input: {
    currentFromTop: number;
    boundaryHeight: number;
    mountedContent: boolean;
  }) => number | null,
): boolean {
  const boundary = el.querySelector<HTMLElement>(
    "[data-transcript-scrollback-fill]",
  );
  if (!boundary) return false;
  const fromBottom = Math.abs(el.scrollTop);
  const currentFromTop = Math.max(0, el.scrollHeight - el.clientHeight - fromBottom);
  const desiredFromTop = computeReplacementFromTop({
    currentFromTop,
    boundaryHeight: boundary.getBoundingClientRect().height,
    mountedContent: true,
  });
  if (desiredFromTop === null) return false;
  const desiredFromBottom = Math.max(
    0,
    el.scrollHeight - el.clientHeight - desiredFromTop,
  );
  const target = columnReverseAwayDirection(el) * desiredFromBottom;
  if (Math.abs(el.scrollTop - target) < 0.5) return false;
  anchor.self = true;
  el.scrollTop = target;
  captureFreezeAnchor(el, anchor);
  return true;
}

/** Measure the Desktop reading step distances (line / half-page / page) from
 *  the transcript container without duplicating geometry arithmetic inline. */
export function desktopTranscriptScrollSteps(el: HTMLElement): {
  line: number;
  halfPage: number;
  page: number;
} {
  const line = Math.max(
    32,
    Number.parseFloat(globalThis.getComputedStyle(el).lineHeight) || 24,
  );
  return {
    line,
    halfPage: Math.max(line, el.clientHeight * 0.5),
    page: Math.max(line, el.clientHeight * 0.9),
  };
}

/** Scroll a column-reverse transcript away from its live edge by `distance`. */
export function scrollColumnReverseAway(
  el: HTMLElement,
  distance: number,
): void {
  el.scrollBy({
    top: columnReverseAwayDirection(el) * distance,
    behavior: "auto",
  });
}

/** Scroll a column-reverse transcript toward its live edge by `distance`.
 *  Returns true when the viewport reached the bottom edge. */
export function scrollColumnReverseTowardLatest(
  el: HTMLElement,
  distance: number,
  anchor: FreezeAnchor,
): boolean {
  if (Math.abs(el.scrollTop) <= distance) {
    pinTranscriptToLatest(el, anchor);
    return true;
  }
  const direction = el.scrollTop > 0 ? -1 : 1;
  el.scrollBy({ top: direction * distance, behavior: "auto" });
  return false;
}

/** Focus and reveal a located transcript row (tool or task) cleanly after any
 *  recycled live window has been expanded. */
export function revealLocatedTranscriptRow(
  el: HTMLElement | null,
  row: HTMLElement | null,
  anchor: FreezeAnchor,
  focusSelector?: string,
): void {
  if (!row) return;
  row.scrollIntoView({ block: "center", behavior: "auto" });
  if (el) captureFreezeAnchor(el, anchor);
  if (focusSelector) {
    row.querySelector<HTMLElement>(focusSelector)?.focus({ preventScroll: true });
  }
}
