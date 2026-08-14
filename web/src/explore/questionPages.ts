import { isHumanPrompt, type RenderItem } from "../derive";

export interface QuestionPage {
  id: string;
  title: string;
  itemKeys: string[];
  questionCount: number;
  startsAt: number;
  endsAt: number;
}

export interface QuestionPageIndexEntry {
  id: string;
  ordinal: number;
}

export interface QuestionPageDirectoryEntry extends QuestionPageIndexEntry {
  title: string;
}

/**
 * Merge the durable question index with the currently hydrated transcript.
 *
 * The index may be a paged suffix while the transcript can contain a live page
 * that has not reached storage yet. Keep one row per durable id and sort by the
 * human ordinal so every directory surface reads oldest-to-newest, top-to-bottom.
 */
export function mergeQuestionPageDirectory(
  indexed: readonly QuestionPageDirectoryEntry[],
  loaded: readonly Pick<QuestionPage, "id" | "title">[],
  total: number,
): QuestionPageDirectoryEntry[] {
  const byId = new Map(indexed.map((page) => [page.id, page]));
  let nextOrdinal = indexed.reduce(
    (highest, page) => Math.max(highest, page.ordinal),
    Math.max(0, total - loaded.length),
  );
  for (const page of loaded) {
    if (byId.has(page.id)) continue;
    nextOrdinal += 1;
    byId.set(page.id, {
      id: page.id,
      title: page.title,
      ordinal: nextOrdinal,
    });
  }
  return [...byId.values()].sort((left, right) => left.ordinal - right.ordinal);
}

/**
 * Project the canonical chronological directory into a visual list.
 *
 * Storage and mobile paging stay oldest-first. Desktop's transient navigator
 * is a recency tool, so it reverses a copy without mutating the canonical
 * array shared by page navigation.
 */
export function presentQuestionPageDirectory<T>(
  pages: readonly T[],
  newestFirst: boolean,
): T[] {
  return newestFirst ? [...pages].reverse() : [...pages];
}

/**
 * Resolve a provisional live-tail page to its durable user-message root.
 *
 * A retained timeline can begin inside a streamed answer. That temporary page
 * is useful while history is arriving, but its first chunk seq is not a valid
 * `/question-pages/:page_id` key and must never become the persisted identity
 * of the completed last page.
 */
export function authoritativeTailPageId(
  current: Pick<QuestionPage, "id" | "questionCount"> | null,
  atTail: boolean,
  indexedPages: readonly QuestionPageIndexEntry[],
): string | null {
  if (!current || current.questionCount > 0 || !atTail) return null;
  return indexedPages.at(-1)?.id ?? null;
}

export function indexedQuestionPagePosition(
  pages: QuestionPageIndexEntry[],
  currentId: string | undefined,
  fallbackOrdinal?: number,
): {
  ordinal: number | undefined;
  previousId: string | undefined;
  nextId: string | undefined;
} {
  const idIndex = currentId
    ? pages.findIndex((page) => page.id === currentId)
    : -1;
  const index = idIndex >= 0
    ? idIndex
    : pages.findIndex((page) => page.ordinal === fallbackOrdinal);
  if (index < 0) {
    return { ordinal: undefined, previousId: undefined, nextId: undefined };
  }
  return {
    ordinal: pages[index]?.ordinal,
    previousId: pages[index - 1]?.id,
    nextId: pages[index + 1]?.id,
  };
}

function messageText(item: RenderItem): string {
  if (item.kind !== "message") return "";
  return item.chunks
    .filter((chunk) => chunk.type === "text")
    .map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("")
    .trim();
}

function isContextManagementCommand(item: RenderItem): boolean {
  if (item.kind !== "message" || item.role !== "user") return false;
  const text = messageText(item);
  return text === "/compact" || text === "/compress";
}

export function questionTitle(item: RenderItem, ordinal: number): string {
  const text = messageText(item)
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^[#>*+\-\d.\s`]+/, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!text) return `Page ${String(ordinal)}`;
  return text.length > 72 ? `${text.slice(0, 69).trimEnd()}…` : text;
}

/**
 * Project canonical render items into stable question pages. Existing history
 * has no page metadata, so every human prompt begins a page. Auto-resume echoes
 * stay inside the preceding page. Explicit merge/split metadata can layer over
 * these deterministic boundaries without changing the event timeline.
 */
export function deriveQuestionPages(items: RenderItem[]): QuestionPage[] {
  const pages: QuestionPage[] = [];
  let current: QuestionPage | null = null;

  for (let index = 0; index < items.length; index += 1) {
    const item = items[index]!;
    const beginsPage = item.kind === "message" && item.role === "user" &&
      isHumanPrompt(item.origin) && !isContextManagementCommand(item);

    if (beginsPage) {
      if (current) {
        current.endsAt = index - 1;
        pages.push(current);
      }
      current = {
        id: item.key,
        title: questionTitle(item, pages.length + 1),
        itemKeys: [item.key],
        questionCount: 1,
        startsAt: index,
        endsAt: index,
      };
      continue;
    }

    // Clear starts a new transcript epoch and cannot belong to an older
    // question. Leaving its divider as a provisional page makes an empty Page
    // View try to restore the divider as question history forever.
    if (!current && item.kind === "cleared") continue;

    // A bounded history snapshot can begin in the middle of an answer while its
    // root user prompt is still on the next older HTTP page. Keep those leading
    // rows addressable as a provisional page instead of dropping them. When the
    // older page arrives, deterministic derivation naturally replaces this
    // provisional boundary with the real question root.
    if (!current) {
      current = {
        id: item.key,
        title: "Earlier page",
        itemKeys: [item.key],
        questionCount: 0,
        startsAt: index,
        endsAt: index,
      };
      continue;
    }

    if (current) {
      current.itemKeys.push(item.key);
      current.endsAt = index;
    }
  }

  if (current) pages.push(current);
  return pages;
}

export function groupQuestionPages(
  pages: QuestionPage[],
  pageParents: Record<string, string>,
): QuestionPage[] {
  if (Object.keys(pageParents).length === 0) return pages;
  const grouped = new Map<string, QuestionPage>();
  const result: QuestionPage[] = [];

  for (const page of pages) {
    const targetId = pageParents[page.id] ?? page.id;
    const target = grouped.get(targetId);
    if (target) {
      target.itemKeys.push(...page.itemKeys);
      target.questionCount += page.questionCount;
      target.endsAt = Math.max(target.endsAt, page.endsAt);
      continue;
    }
    const copy = { ...page, itemKeys: [...page.itemKeys] };
    grouped.set(page.id, copy);
    result.push(copy);
  }
  return result;
}

export function pageContainingItemKey(
  pages: QuestionPage[],
  itemKey: string | null,
): QuestionPage | undefined {
  if (!itemKey) return undefined;
  return pages.find((page) => page.itemKeys.includes(itemKey));
}

/**
 * Resolve the preceding complete question page for history navigation.
 * A bounded history response may expose a provisional leading answer before
 * its user prompt arrives. That placeholder is renderable, but it is not a
 * valid navigation destination: callers must keep loading until derivation
 * attaches those rows to a real user-rooted page.
 */
export function completePageBeforeItem(
  pages: QuestionPage[],
  itemKey: string,
): QuestionPage | undefined {
  const currentIndex = pages.findIndex((page) =>
    page.itemKeys.includes(itemKey)
  );
  const previous = pages[currentIndex - 1];
  return previous?.questionCount ? previous : undefined;
}
