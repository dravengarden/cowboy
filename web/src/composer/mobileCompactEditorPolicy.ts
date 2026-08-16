import { imageTokensInText } from "../attachments";
import { inlineImagePasteInsertion } from "../inlineImageSelection";

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
  const edit = inlineImagePasteInsertion(value, from, to, attachments);
  if (attachments.length === 0) return { value, caret: edit.from };
  return {
    value: value.slice(0, edit.from) + edit.insert + value.slice(edit.to),
    caret: edit.caret,
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
 * Pitfall #69 candidate 2. Physical iOS binds `caretRect` to a replaced
 * `<img>` inside contenteditable, so promoting the native textarea to CM6
 * after paste leaves the painted caret on the thumbnail. New touch pastes
 * stay in the attachment tray; the textarea never gains a placement token.
 * Existing token-bearing documents still promote.
 */
export function touchImagePasteInsertsTokens(): boolean {
  return false;
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

function hasNewImageToken(frozenSeed: string, touchValue: string): boolean {
  const seedIds = new Set(
    imageTokensInText(frozenSeed).map((token) => token.id),
  );
  return imageTokensInText(touchValue).some((token) => !seedIds.has(token.id));
}

/** Freeze the document used to mount uncontrolled CM6. While native is active,
 * track its live controlled text; on the one native → CM6 promotion render,
 * capture the just-inserted image token. Once CM6 owns the editor, follow
 * later React text only when a new inline-image token appears. Ordinary
 * typing/IME and stale token-less echoes must not rewrite the seed:
 * @uiw/react-codemirror resets the document whenever `value` differs from
 * the live doc, which would flash a second pasted thumbnail and then
 * delete it. */
export function composerEditorMountSeed(
  wasNative: boolean,
  nativeNow: boolean,
  frozenSeed: string,
  touchValue: string,
): string {
  if (wasNative || nativeNow) return touchValue;
  return hasNewImageToken(frozenSeed, touchValue) ? touchValue : frozenSeed;
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

/** A focused CM6 editor replaced after its final image token is deleted must
 * hand the existing UIKit keyboard session to the native textarea during that
 * same commit. An unfocused/programmatic document change must not summon it. */
export function shouldFocusDemotedEditor(
  committedNative: boolean,
  nativeNow: boolean,
  cmHadFocus: boolean,
): boolean {
  return !committedNative && nativeNow && cmHadFocus;
}

/** Preserve a forward or backward logical selection across the CM6 -> native
 * replacement. Keep the claim readable through replayed renders until the
 * transition's layout effect confirms that the textarea committed. */
export function nativeDemotionSelection(
  committedNative: boolean,
  nativeNow: boolean,
  selection: { anchor: number; head: number } | null,
): { anchor: number; head: number } | undefined {
  return !committedNative && nativeNow && selection !== null
    ? selection
    : undefined;
}

/** Keep the native caret claim available for every render attempt until the
 * native -> CM6 transition actually commits. React may replay or supersede a
 * render before commit; consuming this value during render strands CM6 at its
 * default selection (document offset 0). */
export function nativePromotionSelection(
  committedNative: boolean,
  nativeNow: boolean,
  caret: number | null,
): number | undefined {
  return committedNative && !nativeNow && caret !== null ? caret : undefined;
}
