// FREEZE-WHILE-DETACHED anchor and column-reverse scroll helpers.
//
// Held in a ref so both the scroll listener (capture) and the per-chunk
// timeline layout effect (restore) share one anchor. `self` flags our own
// corrective scrollTop write so the listener swallows the scroll event it
// triggers.
export interface FreezeAnchor {
  attr: "data-key" | "data-ancestor-item";
  key: string | null;
  top: number;
  self: boolean;
}

const ANCHOR_ROW_SELECTOR = "[data-key], [data-ancestor-item]";

export function createFreezeAnchor(): FreezeAnchor {
  return { attr: "data-key", key: null, top: 0, self: false };
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
 *  from 0 once when at the live edge instead of UA-sniffing. */
export function columnReverseAwayDirection(
  el: HTMLElement,
  anchor?: FreezeAnchor,
): 1 | -1 {
  if (el.scrollTop > 0.5) return 1;
  if (el.scrollTop < -0.5) return -1;
  if (el.scrollHeight <= el.clientHeight + 1) return -1;
  const before = el.scrollTop;
  if (anchor) anchor.self = true;
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
export function unobscuredReadingMidY(
  rect: Pick<DOMRect, "top" | "bottom" | "height">,
  paddingTop: number,
  paddingBottom: number,
): number {
  const safeTop = rect.top + Math.max(0, paddingTop);
  const safeBottom = rect.bottom - Math.max(0, paddingBottom);
  if (safeBottom > safeTop + 8) return (safeTop + safeBottom) / 2;
  return rect.top + rect.height / 2;
}

function readAnchorRow(
  el: HTMLElement,
  row: HTMLElement,
): { attr: "data-key" | "data-ancestor-item"; key: string } | null {
  if (!el.contains(row)) return null;
  const key = row.getAttribute("data-key");
  if (key !== null) return { attr: "data-key", key };
  const ancestorKey = row.getAttribute("data-ancestor-item");
  if (ancestorKey !== null) return { attr: "data-ancestor-item", key: ancestorKey };
  return null;
}

/** Find the best anchor row covering or nearest `midY` when `elementFromPoint`
 *  hits a floating overlay (Mobile Composer / navbar / toast) or inter-row gap. */
export function pickVisibleAnchorRow(
  el: HTMLElement,
  elRect: Pick<DOMRect, "top" | "bottom">,
  midY: number,
): HTMLElement | null {
  let best: HTMLElement | null = null;
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const row of el.querySelectorAll<HTMLElement>(ANCHOR_ROW_SELECTOR)) {
    const r = row.getBoundingClientRect();
    if (r.height <= 0) continue;
    if (r.bottom <= elRect.top || r.top >= elRect.bottom) continue;
    if (r.top <= midY && r.bottom >= midY) return row;
    const distance = r.top > midY ? r.top - midY : midY - r.bottom;
    if (distance < bestDistance) {
      bestDistance = distance;
      best = row;
    }
  }
  return best;
}

/** Snapshot the visible row at the unobscured reading-band centre + its offset
 *  from the container top. Probing inside the unobscured band with a DOM-scan
 *  fallback prevents floating Mobile Composer overlays, inter-row gaps, or
 *  inherited ancestor segments from silently dropping the freeze anchor. */
export function captureFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  const r = el.getBoundingClientRect();
  const style = el.ownerDocument.defaultView?.getComputedStyle(el);
  const padTop = style ? Number.parseFloat(style.paddingTop) || 0 : 0;
  const padBottom = style ? Number.parseFloat(style.paddingBottom) || 0 : 0;
  const midY = unobscuredReadingMidY(r, padTop, padBottom);
  const hit = el.ownerDocument.elementFromPoint(r.left + r.width / 2, midY);
  const hitRow = hit?.closest<HTMLElement>(ANCHOR_ROW_SELECTOR) ?? null;
  const resolved = (hitRow ? readAnchorRow(el, hitRow) : null) ??
    (() => {
      const fallback = pickVisibleAnchorRow(el, r, midY);
      return fallback ? readAnchorRow(el, fallback) : null;
    })();
  if (!resolved) {
    a.key = null;
    return;
  }
  const selector = `[${resolved.attr}="${CSS.escape(resolved.key)}"]`;
  const row = el.querySelector<HTMLElement>(selector);
  if (!row) {
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
 *  idle reader who is still sitting inside the boundary just past it. Returns
 *  true when the viewport was adjusted. */
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
  const direction = columnReverseAwayDirection(el, anchor);
  anchor.self = true;
  el.scrollTop = direction * desiredFromBottom;
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
  anchor: FreezeAnchor,
): void {
  el.scrollBy({
    top: columnReverseAwayDirection(el, anchor) * distance,
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
