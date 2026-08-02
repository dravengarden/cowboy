// Clipboard API for secure production origins, with a legacy fallback for the
// plain-HTTP development bridge where navigator.clipboard is unavailable.
export async function copyText(text: string): Promise<boolean> {
  try {
    if (globalThis.navigator?.clipboard?.writeText) {
      await globalThis.navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // A denied or unfocused secure context can still use the fallback below.
  }
  try {
    const doc = globalThis.document;
    const textarea = doc.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    doc.body.appendChild(textarea);
    textarea.select();
    // eslint-disable-next-line @typescript-eslint/no-deprecated -- HTTP dev fallback
    const copied = doc.execCommand("copy");
    textarea.remove();
    return copied;
  } catch {
    return false;
  }
}

interface ReadableClipboardItem {
  readonly types: readonly string[];
  getType(type: string): Promise<Blob>;
}

interface ReadableClipboard {
  read?: () => Promise<readonly ReadableClipboardItem[]>;
  readText?: () => Promise<string>;
}

export interface ComposerClipboardContent {
  text: string;
  files: File[];
}

function clipboardFileName(type: string, index: number): string {
  const subtype = type.split("/", 2)[1]?.split(";", 1)[0] || "bin";
  const suffix = index === 0 ? "" : `-${String(index + 1)}`;
  return `pasted-file${suffix}.${subtype}`;
}

/** Read a trusted-user-gesture clipboard for the Mobile blank-canvas Paste
 * fallback. Rich text keeps its plain-text representation; each item contributes
 * at most one binary representation so image/png + text/html is not duplicated. */
export async function readComposerClipboard(
  clipboard: ReadableClipboard | undefined = globalThis.navigator?.clipboard,
): Promise<ComposerClipboardContent> {
  if (!clipboard) throw new Error("Clipboard access is unavailable");

  if (clipboard.read) {
    const items = await clipboard.read();
    const files: File[] = [];
    const textParts: string[] = [];
    for (const item of items) {
      const binaryType = item.types.find((type) => type.startsWith("image/")) ??
        item.types.find((type) => !type.startsWith("text/"));
      if (binaryType) {
        const blob = await item.getType(binaryType);
        files.push(
          new File([blob], clipboardFileName(binaryType, files.length), {
            type: binaryType,
          }),
        );
        continue;
      }
      if (item.types.includes("text/plain")) {
        textParts.push(await (await item.getType("text/plain")).text());
      }
    }
    if (files.length > 0 || textParts.length > 0) {
      return { text: textParts.join("\n"), files };
    }
  }

  if (clipboard.readText) {
    return { text: await clipboard.readText(), files: [] };
  }
  throw new Error("Clipboard access is unavailable");
}
