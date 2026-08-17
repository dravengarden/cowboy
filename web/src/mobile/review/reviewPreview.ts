import { isMarkdownReviewPath } from "./reviewMarkdown.ts";

export type ReviewPreviewKind = "text" | "markdown" | "image" | "svg" | "mermaid";

const IMAGE_EXTENSION = /\.(?:png|jpe?g|gif|webp|bmp|avif|ico)$/i;
const SVG_EXTENSION = /\.svg$/i;
const MERMAID_EXTENSION = /\.(?:mmd|mermaid)$/i;

export function reviewPreviewKind(path: string): ReviewPreviewKind {
  if (IMAGE_EXTENSION.test(path)) return "image";
  if (SVG_EXTENSION.test(path)) return "svg";
  if (MERMAID_EXTENSION.test(path)) return "mermaid";
  if (isMarkdownReviewPath(path)) return "markdown";
  return "text";
}

export function isReviewMediaPath(path: string): boolean {
  const kind = reviewPreviewKind(path);
  return kind === "image" || kind === "svg";
}

export function reviewMediaUrl(sessionId: string, path: string): string {
  const query = new URLSearchParams({ path });
  return `/api/code/sessions/${encodeURIComponent(sessionId)}/file-raw?${query}`;
}
