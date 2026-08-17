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
 * Line-start markup that Obsidian live-preview would hide on inactive lines.
 * `# hi` / `- 主题` must promote off the native textarea or they stay raw.
 */
const TOUCH_LIVE_PREVIEW_MARKUP =
  /^(?:#{1,6} |\s*[-*+] (?:\[[ xX]\] )?|\s*\d+\. |\s*> |```)/m;

export function hasTouchLivePreviewMarkup(value: string): boolean {
  return TOUCH_LIVE_PREVIEW_MARKUP.test(value);
}

/**
 * Native iOS text controls own the reliable long-press edit menu. CM6 owns
 * Obsidian live preview and inline-image widgets. Keep the native control
 * for plain token-free prose; the first heading/list/quote/fence or image
 * token promotes the same literal document to CM6.
 */
export function shouldUseNativeTouchEditor(
  surfaceKind: "desktop" | "tablet" | "mobile",
  value: string,
): boolean {
  if (surfaceKind === "desktop") return false;
  if (imageTokensInText(value).length > 0) return false;
  return !hasTouchLivePreviewMarkup(value);
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
