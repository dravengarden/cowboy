import { imageTokensInText } from "../attachments";

interface NativeInlineImage {
  id: string;
  name: string;
}

export interface NativeInlineImageEdit {
  value: string;
  caret: number;
}

/** Insert one or more placement tokens into the live native-textarea document.
 * The returned caret is part of the edit: the textarea is replaced by CM6 as
 * soon as the first token lands, so there is no surviving native selection to
 * query on the promotion render. */
export function insertNativeInlineImages(
  value: string,
  from: number,
  to: number,
  attachments: readonly NativeInlineImage[],
): NativeInlineImageEdit {
  const start = Math.max(0, Math.min(from, value.length));
  const end = Math.max(start, Math.min(to, value.length));
  if (attachments.length === 0) return { value, caret: start };
  const lineStart = value.lastIndexOf("\n", start - 1) + 1;
  const tokens = attachments.map((attachment, index) => {
    const lead = index === 0 && start !== lineStart ? "\n" : "";
    const label = attachment.name.replaceAll("]", "");
    return `${lead}![${label}](cowboy-att:${attachment.id})\n`;
  }).join("");
  return {
    value: value.slice(0, start) + tokens + value.slice(end),
    caret: start + tokens.length,
  };
}

/** The persisted inline-height preference belongs to Desktop only. Mobile and
 * tablet have a separate, explicit fullscreen sheet; leaking the Desktop bit
 * into their compact composer silently promotes it to a 48vh CM6 canvas. */
export function shouldExpandInlineComposer(
  surfaceKind: "desktop" | "tablet" | "mobile",
  expandedPreference: boolean,
): boolean {
  return surfaceKind === "desktop" && expandedPreference;
}

/**
 * Native iOS text controls own the reliable long-press edit menu. CM6 owns
 * inline-image widgets. Use the native control on every touch writing surface
 * while the document has no image placement token; the first token promotes
 * the same literal document to CM6 without moving or deleting the image.
 */
export function shouldUseNativeTouchEditor(
  surfaceKind: "desktop" | "tablet" | "mobile",
  value: string,
): boolean {
  return surfaceKind !== "desktop" && imageTokensInText(value).length === 0;
}

/** Freeze the document used to mount uncontrolled CM6. While native is active,
 * track its live controlled text; on the one native → CM6 promotion render,
 * capture the just-inserted image token. Once CM6 owns the editor, never follow
 * later React text updates or IME/caret reconciliation regresses. */
export function composerEditorMountSeed(
  wasNative: boolean,
  nativeNow: boolean,
  frozenSeed: string,
  touchValue: string,
): string {
  return wasNative || nativeNow ? touchValue : frozenSeed;
}

/** A focused native textarea replaced during native → CM6 promotion must hand
 * its UIKit keyboard session to the new editor in the same React commit. */
export function shouldFocusPromotedEditor(
  wasNative: boolean,
  nativeNow: boolean,
  activeElementIsTextarea: boolean,
  pastePromotionPending = false,
): boolean {
  return wasNative && !nativeNow &&
    (activeElementIsTextarea || pastePromotionPending);
}
