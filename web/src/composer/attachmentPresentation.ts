import { type Attachment, imageTokensInText } from "../attachments";

// CM6 renders token-backed images inline on every surface. The tray contains
// ordinary files and legacy/unplaced images only, never a second rendering of a
// token-backed image.
export function attachmentTrayForSurface(
  attachments: readonly Attachment[],
  text: string,
): Attachment[] {
  const inlineImageIds = new Set(imageTokensInText(text).map((token) => token.id));
  return attachments.filter((attachment) =>
    !attachment.isImage ||
    !inlineImageIds.has(attachment.id)
  );
}
