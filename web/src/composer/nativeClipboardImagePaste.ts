import {
  type Attachment,
  fileToAttachment,
  pendingClipboardImageAttachment,
  pendingImageAttachment,
} from "../attachments";
import type { ComposerEditorSelection } from "./PlatformComposerEditor";

const MAX_NATIVE_CLIPBOARD_IMAGES = 100;

export interface NativeClipboardImagePasteRequest {
  expectedCount: number;
  selection: ComposerEditorSelection;
  /** Starts the privacy-gated native payload read. The host must stage first. */
  read: () => Promise<File[]>;
}

export interface NativeClipboardImagePasteHost {
  /** Must insert these pending tokens synchronously before returning. */
  stage: (
    pending: Attachment[],
    selection?: ComposerEditorSelection,
  ) => void;
  /** Replaces this invocation's pending ids and removes only its failures. */
  settle: (pending: Attachment[], completed: Attachment[]) => void;
}

export function nativeClipboardPlaceholderCount(expectedCount: number): number {
  if (!Number.isFinite(expectedCount)) return 1;
  return Math.max(
    1,
    Math.min(MAX_NATIVE_CLIPBOARD_IMAGES, Math.trunc(expectedCount)),
  );
}

/**
 * Own one explicit native clipboard request. The first stage runs before the
 * payload bridge is invoked, keeping textarea -> CM6 promotion inside the
 * originating UIKit tap. Encoding and provider loading can then finish without
 * owning focus. A stale metadata count is repaired by adding/removing only this
 * invocation's ids; overlapping paste batches therefore cannot delete each
 * other.
 */
export async function runNativeClipboardImagePaste(
  request: NativeClipboardImagePasteRequest,
  host: NativeClipboardImagePasteHost,
): Promise<void> {
  const initialPending = Array.from(
    { length: nativeClipboardPlaceholderCount(request.expectedCount) },
    (_, index) => pendingClipboardImageAttachment(index),
  );
  host.stage(initialPending, request.selection);

  let files: File[];
  try {
    files = (await request.read()).filter((file) =>
      (file.type || "").startsWith("image/")
    ).slice(0, MAX_NATIVE_CLIPBOARD_IMAGES);
  } catch {
    files = [];
  }

  // Metadata and payload use the same pasteboard change count in the native
  // shell, but the clipboard can still change between the poll and the tap.
  // The initial placeholder keeps focus; any extra payloads can now be inserted
  // into the already-mounted CM6 editor without another UIKit focus transfer.
  const extraPending = files.slice(initialPending.length).map((file) =>
    pendingImageAttachment(file)
  );
  if (extraPending.length > 0) host.stage(extraPending);
  const pending = [...initialPending, ...extraPending];

  const settled = await Promise.allSettled(
    files.map((file, index) => fileToAttachment(file, pending[index]!.id)),
  );
  const completed = settled.flatMap((result) =>
    result.status === "fulfilled" ? [result.value] : []
  );
  try {
    host.settle(pending, completed);
  } finally {
    pending.forEach((attachment) => {
      if (attachment.previewUrl?.startsWith("blob:")) {
        URL.revokeObjectURL(attachment.previewUrl);
      }
    });
  }
}
