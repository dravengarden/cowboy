/** Touch-wrapped Markdown (Review README/documents) must not contain nested
 *  horizontal ScrollViews. iOS WebKit backs every overflowing `overflow-x:
 *  auto` box with its own UIScrollView, visible or not, and re-commits each
 *  one on every translate3d frame of the drawer/pager peek. A handbook with
 *  fifty wide tables therefore hitches while an equally long prose README
 *  stays silky. It also makes a swipe that starts on a table a native table
 *  pan instead of a workspace swipe (`hasHorizontalScroller`).
 *
 *  Fit mode sizes each table to the column instead. Cells wrap at word
 *  boundaries first. A table whose longest words still cannot fit steps
 *  down to a compact font, and only then breaks anywhere, which always
 *  fits: breaking short terms mid-word is the last resort. The wrapper clips
 *  (never scrolls), so an unexpected overflow cannot give the whole Review
 *  document a horizontal range that would disable the workspace swipe. */

export const MARKDOWN_TABLE_WRAP_ATTRIBUTE = "data-markdown-table-wrap";
export type MarkdownTableWrap = "compact" | "anywhere";

const FIT_TOLERANCE_PX = 1;

export function markdownTableOverflows(
  tableWidth: number,
  availableWidth: number,
): boolean {
  return tableWidth > availableWidth + FIT_TOLERANCE_PX;
}

export const markdownTableFitSx = {
  "& [data-markdown-table-scroll]": {
    // `clip` is not a scroll container: no UIScrollView, and not a
    // horizontal scroller for the drawer/pager recognizers.
    overflowX: "clip",
    WebkitOverflowScrolling: "auto",
  },
  "& [data-markdown-table-scroll] > table": { width: "auto" },
  "& [data-markdown-table-scroll] > table :is(th, td)": {
    whiteSpace: "normal",
    overflowWrap: "break-word",
    wordBreak: "normal",
  },
  [`& [data-markdown-table-scroll] > table[${MARKDOWN_TABLE_WRAP_ATTRIBUTE}]`]: {
    fontSize: "0.85em",
  },
  [`& [data-markdown-table-scroll] > table[${MARKDOWN_TABLE_WRAP_ATTRIBUTE}='anywhere'] :is(th, td)`]:
    {
      overflowWrap: "anywhere",
    },
} as const;

interface FitTable {
  removeAttribute: (name: string) => void;
  setAttribute: (name: string, value: string) => void;
  readonly offsetWidth: number;
  readonly parentElement: { readonly clientWidth: number } | null;
}

function overflowing(tables: readonly FitTable[]): FitTable[] {
  return tables.filter((table) =>
    table.parentElement !== null &&
    markdownTableOverflows(table.offsetWidth, table.parentElement.clientWidth)
  );
}

/** Batched write/read phases per step: at most three layouts of the
 *  document however many tables it contains. */
export function fitMarkdownTables(tables: readonly FitTable[]): void {
  for (const table of tables) {
    table.removeAttribute(MARKDOWN_TABLE_WRAP_ATTRIBUTE);
  }
  let pending = overflowing(tables);
  for (const wrap of ["compact", "anywhere"] as const satisfies readonly MarkdownTableWrap[]) {
    if (pending.length === 0) return;
    for (const table of pending) {
      table.setAttribute(MARKDOWN_TABLE_WRAP_ATTRIBUTE, wrap);
    }
    if (wrap !== "anywhere") pending = overflowing(pending);
  }
}

function markdownTables(root: HTMLElement): HTMLTableElement[] {
  return Array.from(
    root.querySelectorAll<HTMLTableElement>(
      "[data-markdown-table-scroll] > table",
    ),
  );
}

/** Fit now, and again only when the column width changes (rotation, split
 *  view). A drawer or pager translate never changes layout width, so the
 *  swipe path never runs this. The refit is deferred out of the observer
 *  callback so re-wrapping heights cannot raise a ResizeObserver loop. */
export function bindMarkdownTableFit(root: HTMLElement): () => void {
  fitMarkdownTables(markdownTables(root));
  if (typeof ResizeObserver !== "function") return () => undefined;
  let width = root.clientWidth;
  let frame = 0;
  const observer = new ResizeObserver((entries) => {
    const next = entries[entries.length - 1]?.contentRect.width ?? width;
    if (Math.abs(next - width) < 0.5) return;
    width = next;
    if (frame !== 0) cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      frame = 0;
      fitMarkdownTables(markdownTables(root));
    });
  });
  observer.observe(root);
  return () => {
    observer.disconnect();
    if (frame !== 0) cancelAnimationFrame(frame);
  };
}
