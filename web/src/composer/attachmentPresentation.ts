import { type Attachment, imageTokensInText } from "../attachments";

export type ComposerAttachmentSurface =
  | "native-compact"
  | "cm-compact"
  | "cm-fullscreen";

// CM6 renders token-backed images inline. A native textarea cannot, so every
// staged attachment belongs in its tray. Fullscreen CM6 may inherit images
// pasted in the native composer; keep only those unplaced images in the tray.
export function attachmentTrayForSurface(
  attachments: readonly Attachment[],
  text: string,
  surface: ComposerAttachmentSurface,
): Attachment[] {
  if (surface === "native-compact") return [...attachments];
  const inlineImageIds = surface === "cm-fullscreen"
    ? new Set(imageTokensInText(text).map((token) => token.id))
    : undefined;
  return attachments.filter((attachment) =>
    !attachment.isImage ||
    (inlineImageIds !== undefined && !inlineImageIds.has(attachment.id))
  );
}
