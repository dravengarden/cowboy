import { useCallback, useEffect, useState } from "react";
import { type ComposerEditorHandle } from "./composer/PlatformComposerEditor";
import { dismissMobileSoftwareKeyboard, releaseMobileComposerFocus, noteMobileKeyboardDismissed } from "./composer/mobileComposerFocus";
import { isImeKeyEvent } from "./imeKey";
import { openLightbox } from "./ResourceLightbox";
import { type Attachment } from "./attachments";
import { getInlineAttachment, setImageTapHandler } from "./inlineImages";
import { useReliableTouchTap } from "./useReliableTouchTap";

/** Inline-image selection popover. Tapping an inline image opens a small
 *  popover (Preview / Delete) anchored to its <img>, ringed while open. */
export function useInlineImageSelection({
  attachments,
  setAttachments,
  editorRef,
  setMobileKeyboardDismissed,
}: {
  attachments: Attachment[];
  setAttachments: React.Dispatch<React.SetStateAction<Attachment[]>>;
  editorRef: React.RefObject<ComposerEditorHandle | null>;
  setMobileKeyboardDismissed: (dismissed: boolean) => void;
}) {
  // Inline-image selection popover. Tapping an inline image opens a small popover
  // (Preview / Delete) anchored to its <img>, ringed while open. The image widget
  // lives outside React, so it calls a module-level tap handler we register here.
  const [imgSel, setImgSel] = useState<
    { id: string; el: HTMLElement; x: number; y: number } | null
  >(null);
  const closeImgSel = useCallback((): void => {
    setImgSel((cur) => {
      cur?.el.classList.remove("cm-inline-image-selected");
      return null;
    });
  }, []);
  const previewSelectedImage = useCallback((): void => {
    // Resolve from the INLINE-IMAGE REGISTRY (all surfaces register there),
    // not the local `attachments` — otherwise Preview no-ops in the expanded
    // editor whose image lives in editAttachments.
    const id = imgSel?.id;
    const att = (id ? getInlineAttachment(id) : undefined) ??
      attachments.find((attachment) => attachment.id === id);
    // Preview leaves writing for the lightbox, so collapse two-track chrome
    // and drop the software keyboard instead of leaving ∧∨✓ over the image.
    noteMobileKeyboardDismissed();
    setMobileKeyboardDismissed(true);
    dismissMobileSoftwareKeyboard();
    releaseMobileComposerFocus();
    if (att) openLightbox([att], 0);
    closeImgSel();
  }, [attachments, closeImgSel, imgSel]);
  const deleteSelectedImage = useCallback((): void => {
    if (imgSel) {
      editorRef.current?.deleteImage(imgSel.id);
      setAttachments((previous) =>
        previous.filter((attachment) => attachment.id !== imgSel.id)
      );
    }
    closeImgSel();
  }, [closeImgSel, imgSel, setAttachments]);
  // Commit stationary image actions on pointerup while retaining the native
  // click as an independent WebKit fallback. Preventing the TOUCH pointerdown
  // suppresses that fallback; prevent only its compatibility mousedown below
  // so the editor keeps focus without making pointerup the sole activation path.
  const imagePreviewTap = useReliableTouchTap<HTMLButtonElement>(
    previewSelectedImage,
  );
  const imageDeleteTap = useReliableTouchTap<HTMLButtonElement>(
    deleteSelectedImage,
  );
  useEffect(() => {
    setImageTapHandler((id, el, x, y) => {
      setImgSel((prev) => {
        prev?.el.classList.remove("cm-inline-image-selected");
        el.classList.add("cm-inline-image-selected");
        return { id, el, x, y };
      });
    });
    return (): void => setImageTapHandler(null);
  }, []);
  useEffect(() => {
    if (imgSel === null) return undefined;
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape" || isImeKeyEvent(event)) return;
      event.preventDefault();
      event.stopPropagation();
      closeImgSel();
    };
    globalThis.addEventListener("keydown", closeOnEscape, true);
    return (): void =>
      globalThis.removeEventListener("keydown", closeOnEscape, true);
  }, [closeImgSel, imgSel]);
  return { imgSel, setImgSel, closeImgSel, imagePreviewTap, imageDeleteTap };
}
