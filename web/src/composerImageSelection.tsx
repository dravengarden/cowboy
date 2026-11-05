import { useCallback, useEffect, useState } from "react";
import { alpha, Button, ClickAwayListener, Divider, Paper, Popper, Stack } from "@mui/material";
import { DeleteOutline, Visibility } from "@mui/icons-material";
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

const INLINE_IMAGE_ACTION_BUTTON_SX = {
  textTransform: "none",
  fontWeight: 600,
  gap: 0.5,
  px: 1.75,
  py: 1,
  borderRadius: 0,
  "& .MuiButton-startIcon > *": { fontSize: "1.125rem" },
} as const;

/** The Preview / Delete toolbar for the selected inline image. Popper keeps
 *  the active editor and native selection alive while positioning it by the
 *  image. */
export function InlineImageActions({
  imgSel,
  closeImgSel,
  imagePreviewTap,
  imageDeleteTap,
}: Pick<
  ReturnType<typeof useInlineImageSelection>,
  "imgSel" | "closeImgSel" | "imagePreviewTap" | "imageDeleteTap"
>): React.JSX.Element {
  return (
  <Popper
    open={imgSel !== null}
    anchorEl={imgSel?.el ?? null}
    placement="top"
    modifiers={[
      { name: "offset", options: { offset: [0, 8] } },
      {
        name: "flip",
        options: { fallbackPlacements: ["bottom", "right", "left"] },
      },
      { name: "preventOverflow", options: { padding: 8 } },
    ]}
    sx={{ zIndex: (t) => t.zIndex.tooltip }}
  >
    <ClickAwayListener onClickAway={closeImgSel}>
      <Paper
        role="toolbar"
        aria-label="Image actions"
        sx={{
          borderRadius: 999,
          overflow: "hidden",
          border: 1,
          borderColor: "divider",
          boxShadow: 8,
          backgroundImage: "none",
          bgcolor: (t) => alpha(
            t.palette.background.paper,
            t.palette.mode === "dark" ? 0.74 : 0.82,
          ),
          backdropFilter: "blur(16px) saturate(180%)",
          WebkitBackdropFilter: "blur(16px) saturate(180%)",
        }}
      >
        <Stack
          direction="row"
          alignItems="stretch"
          divider={
            <Divider orientation="vertical" flexItem sx={{ my: 0.875 }} />
          }
        >
          <Button
            color="inherit"
            startIcon={<Visibility />}
            onPointerDown={imagePreviewTap.onPointerDown}
            onPointerMove={imagePreviewTap.onPointerMove}
            onPointerUp={imagePreviewTap.onPointerUp}
            onPointerCancel={imagePreviewTap.onPointerCancel}
            onMouseDown={(event): void => event.preventDefault()}
            onClick={imagePreviewTap.onClick}
            sx={INLINE_IMAGE_ACTION_BUTTON_SX}
          >
            Preview
          </Button>
          <Button
            color="error"
            startIcon={<DeleteOutline />}
            onPointerDown={imageDeleteTap.onPointerDown}
            onPointerMove={imageDeleteTap.onPointerMove}
            onPointerUp={imageDeleteTap.onPointerUp}
            onPointerCancel={imageDeleteTap.onPointerCancel}
            onMouseDown={(event): void => event.preventDefault()}
            onClick={imageDeleteTap.onClick}
            sx={INLINE_IMAGE_ACTION_BUTTON_SX}
          >
            Delete
          </Button>
        </Stack>
      </Paper>
    </ClickAwayListener>
  </Popper>
  );
}
