import { type DragEvent, useRef, useState } from "react";
import { clipboardFiles } from "../attachments";

/** A drag from the OS file manager advertises `Files` in `types` for its whole
 *  lifetime, while `files` itself stays empty until `drop`. Text or link drags
 *  keep their native editor behaviour. */
export function dataTransferCarriesFiles(
  transfer: Pick<DataTransfer, "types"> | null | undefined,
): boolean {
  return transfer != null && Array.from(transfer.types).includes("Files");
}

export interface ComposerFileDropTarget {
  /** A file drag is currently over the target. */
  active: boolean;
  handlers: {
    onDragEnter: (event: DragEvent<HTMLElement>) => void;
    onDragOver: (event: DragEvent<HTMLElement>) => void;
    onDragLeave: (event: DragEvent<HTMLElement>) => void;
    onDrop: (event: DragEvent<HTMLElement>) => void;
  };
}

/** Accept files dropped anywhere on a desktop composer card.
 *
 *  The editor claims drops over its own text first (inserting at the drop
 *  point) and prevents the default; this target then only resets its overlay.
 *  Drops on the card's toolbar or padding attach at the current caret. Every
 *  file drag over the card is claimed so the browser never navigates away to
 *  the dropped file. */
export function useComposerFileDrop(
  enabled: boolean,
  onFiles: (files: File[]) => void,
): ComposerFileDropTarget {
  const [active, setActive] = useState(false);
  // dragenter/dragleave fire for every child crossed; only the outermost pair
  // ends the hover.
  const depth = useRef(0);
  const reset = (): void => {
    depth.current = 0;
    setActive(false);
  };
  return {
    active: enabled && active,
    handlers: {
      onDragEnter: (event): void => {
        if (!enabled || !dataTransferCarriesFiles(event.dataTransfer)) return;
        event.preventDefault();
        depth.current += 1;
        setActive(true);
      },
      onDragOver: (event): void => {
        if (!enabled || !dataTransferCarriesFiles(event.dataTransfer)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
      },
      onDragLeave: (event): void => {
        if (!enabled || !dataTransferCarriesFiles(event.dataTransfer)) return;
        depth.current = Math.max(0, depth.current - 1);
        if (depth.current === 0) setActive(false);
      },
      onDrop: (event): void => {
        if (!enabled || !dataTransferCarriesFiles(event.dataTransfer)) return;
        reset();
        if (event.defaultPrevented) return;
        event.preventDefault();
        const files = clipboardFiles(event.dataTransfer);
        if (files.length > 0) onFiles(files);
      },
    },
  };
}
