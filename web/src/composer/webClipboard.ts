// Browser Clipboard API backend. Safari / installed PWA can read after a
// user gesture; they cannot probe the pasteboard the way the native shell
// can. Callers must treat status as "the action may be offered", not
// "we already know there is an image".

export interface WebClipboardContents {
  text: string;
  files: File[];
}

export function supportsWebClipboardRead(
  clipboard: Pick<Clipboard, "read" | "readText"> | undefined =
    globalThis.navigator?.clipboard,
): boolean {
  return typeof clipboard?.read === "function" ||
    typeof clipboard?.readText === "function";
}

function extensionForMime(mimeType: string): string {
  const subtype = mimeType.split("/")[1]?.split(";")[0] ?? "png";
  if (subtype === "jpeg") return "jpg";
  if (subtype === "svg+xml") return "svg";
  return subtype;
}

export async function readWebClipboard(
  clipboard: Pick<Clipboard, "read" | "readText"> | undefined =
    globalThis.navigator?.clipboard,
): Promise<WebClipboardContents> {
  if (!clipboard) return { text: "", files: [] };

  const files: File[] = [];
  let text = "";

  if (typeof clipboard.read === "function") {
    try {
      const items = await clipboard.read();
      let imageIndex = 0;
      for (const item of items) {
        for (const type of item.types) {
          if (!type.startsWith("image/")) continue;
          const blob = await item.getType(type);
          imageIndex += 1;
          files.push(
            new File(
              [blob],
              `pasted-image-${String(imageIndex)}.${extensionForMime(type)}`,
              { type },
            ),
          );
        }
        if (item.types.includes("text/plain")) {
          text = await (await item.getType("text/plain")).text();
        }
      }
      if (files.length > 0 || text.length > 0) return { text, files };
    } catch {
      // NotAllowedError, or Safari builds that expose read() but reject it.
    }
  }

  if (typeof clipboard.readText === "function") {
    try {
      text = await clipboard.readText();
    } catch {
      text = "";
    }
  }
  return { text, files };
}
