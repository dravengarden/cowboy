import type { RenderItem } from "../derive";

export interface QuestionPage {
  id: string;
  title: string;
  itemKeys: string[];
  questionCount: number;
  startsAt: number;
  endsAt: number;
}

function messageText(item: RenderItem): string {
  if (item.kind !== "message") return "";
  return item.chunks
    .filter((chunk) => chunk.type === "text")
    .map((chunk) => chunk.type === "text" ? chunk.text : "")
    .join("")
    .trim();
}

export function questionTitle(item: RenderItem, ordinal: number): string {
  const text = messageText(item)
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "")
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/^[#>*+\-\d.\s`]+/, "")
    .replace(/\s+/g, " ")
    .trim();
  if (!text) return `Question ${String(ordinal)}`;
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
      item.autoResumed !== true;

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

    // A bounded history snapshot can begin in the middle of an answer while its
    // root user prompt is still on the next older HTTP page. Keep those leading
    // rows addressable as a provisional page instead of dropping them. When the
    // older page arrives, deterministic derivation naturally replaces this
    // provisional boundary with the real question root.
    if (!current) {
      current = {
        id: item.key,
        title: "Earlier question",
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
