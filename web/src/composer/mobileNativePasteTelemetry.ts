import { clipboardFiles } from "../attachments";
import { flushObservability, reportClientLog } from "../observability";

export interface MobileNativePasteInventory {
  listedFiles: number;
  itemCount: number;
  itemFiles: number;
  fileCount: number;
}

export interface MobileNativePasteReport {
  surface: "textarea" | "cm6";
  clipboard: Pick<DataTransfer, "files" | "items"> | null;
  fileCount: number;
  consumed: boolean;
}

/** Count paste-event representations without reading bytes or names. */
export function mobileNativePasteInventory(
  clipboard: Pick<DataTransfer, "files" | "items"> | null,
): MobileNativePasteInventory {
  if (!clipboard) {
    return { listedFiles: 0, itemCount: 0, itemFiles: 0, fileCount: 0 };
  }
  const items = Array.from(clipboard.items);
  return {
    listedFiles: clipboard.files.length,
    itemCount: items.length,
    itemFiles: items.filter((item) => item.kind === "file").length,
    fileCount: clipboardFiles(clipboard).length,
  };
}

function focusTag(): string {
  const active = globalThis.document?.activeElement;
  if (!(active instanceof HTMLElement)) return "none";
  if (active.classList.contains("cm-content")) return "cm-content";
  return active.tagName.toLowerCase();
}

/**
 * Bounded, content-free telemetry for the UIKit long-press paste event.
 * Accessory-button paste uses `mobile_paste_*` instead. Attribute keys must
 * stay off the observability denylist (`clipboard` is stripped).
 */
export function reportMobileNativePasteEvent(
  report: MobileNativePasteReport,
): void {
  const inventory = mobileNativePasteInventory(report.clipboard);
  reportClientLog(
    "info",
    "mobile_native_paste_event",
    "Mobile composer received a native paste event",
    {
      surface: report.surface,
      listed_files: inventory.listedFiles,
      item_count: inventory.itemCount,
      item_files: inventory.itemFiles,
      file_count: report.fileCount,
      consumed: report.consumed,
      focus_tag: focusTag(),
    },
  );
  void flushObservability();
}
