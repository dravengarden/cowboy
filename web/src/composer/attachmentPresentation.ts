import { type Attachment, imageTokensInText, isLoadablePreviewUrl } from "../attachments";

// CM6 renders token-backed images inline on every surface. The tray contains
// ordinary files, legacy/unplaced images, and token-backed images whose preview
// cannot paint (empty data URL, HEIC, missing bytes). Hiding those last ones
// from the tray is how a parked draft showed only the "i" and a blank hole.
export function attachmentTrayForSurface(
  attachments: readonly Attachment[],
  text: string,
): Attachment[] {
  const inlineImageIds = new Set(imageTokensInText(text).map((token) => token.id));
  return attachments.filter((attachment) =>
    !attachment.isImage ||
    !inlineImageIds.has(attachment.id) ||
    !isLoadablePreviewUrl(attachment.previewUrl)
  );
}
