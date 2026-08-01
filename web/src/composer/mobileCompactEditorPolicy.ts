import { imageTokensInText } from "../attachments";

/**
 * Native iOS text controls own the reliable long-press edit menu. CM6 owns
 * inline-image widgets. Use the native control only while the compact touch
 * document has no image placement token; the first token promotes the same
 * literal document to CM6 without moving or deleting the image.
 */
export function shouldUseNativeCompactEditor(
  surfaceKind: "desktop" | "tablet" | "mobile",
  expanded: boolean,
  fill: boolean,
  value: string,
): boolean {
  return surfaceKind !== "desktop" && !expanded && !fill &&
    imageTokensInText(value).length === 0;
}
