// Did the native Tauri layer install WKWebView keyboard resizing?
//
// `CowboyNativeTweaks.mm` injects this dedicated flag at document-start only
// after installing native keyboard avoidance. The editor itself now has one
// normal-flow implementation; the remaining consumer uses the flag to avoid
// applying a browser visualViewport inset on top of the native resize.
//
// Do not substitute a generic `__TAURI__` or haptics flag: a shell can provide
// those without providing the keyboard contract. See
// work-items/archive/2026/07/cowboy-native-keyboard-ime.
export function isNativeShell(): boolean {
  if (typeof window === "undefined") return false;
  return (window as { __cowboyNativeShell?: boolean }).__cowboyNativeShell === true;
}

interface NativeClipboardImageStatusPayload {
  hasImages?: unknown;
  imageCount?: unknown;
  changeCount?: unknown;
}

interface NativeClipboardImagePayload {
  name?: unknown;
  mimeType?: unknown;
  data?: unknown;
}

interface NativeClipboardImagesPayload {
  images?: unknown;
  changeCount?: unknown;
}

interface CowboyNativeClipboardWindow extends Window {
  __cowboyClipboardImageStatus?: () => Promise<unknown>;
  __cowboyReadClipboardImages?: () => Promise<unknown>;
}

export interface NativeClipboardImageStatus {
  supported: boolean;
  hasImages: boolean;
  imageCount: number;
  changeCount: number;
}

function nativeClipboardWindow(): CowboyNativeClipboardWindow {
  // `globalThis` is the Window in browsers and remains harmless in Deno/SSR,
  // where the injected bridge properties are simply absent.
  return globalThis as unknown as CowboyNativeClipboardWindow;
}

/** Whether this shell can probe image availability without reading payloads. */
export function supportsNativeClipboardImages(): boolean {
  const root = nativeClipboardWindow();
  return typeof root.__cowboyClipboardImageStatus === "function" &&
    typeof root.__cowboyReadClipboardImages === "function";
}

/** Metadata-only pasteboard probe used to render the exact disabled state. */
export async function nativeClipboardImageStatus(): Promise<NativeClipboardImageStatus> {
  const root = nativeClipboardWindow();
  const bridge = root.__cowboyClipboardImageStatus;
  if (
    typeof bridge !== "function" ||
    typeof root.__cowboyReadClipboardImages !== "function"
  ) {
    return {
      supported: false,
      hasImages: false,
      imageCount: 0,
      changeCount: -1,
    };
  }
  try {
    const raw = await bridge() as NativeClipboardImageStatusPayload | null;
    return {
      supported: true,
      hasImages: raw?.hasImages === true,
      imageCount: typeof raw?.imageCount === "number" &&
          Number.isSafeInteger(raw.imageCount) && raw.imageCount > 0
        ? Math.min(raw.imageCount, 100)
        : raw?.hasImages === true
        ? 1
        : 0,
      changeCount: typeof raw?.changeCount === "number" &&
          Number.isSafeInteger(raw.changeCount)
        ? raw.changeCount
        : -1,
    };
  } catch {
    return {
      supported: true,
      hasImages: false,
      imageCount: 0,
      changeCount: -1,
    };
  }
}

/** Convert the explicit native clipboard reply into ordinary Web Files. */
export function nativeClipboardImageFiles(raw: unknown): File[] {
  const payload = raw as NativeClipboardImagesPayload | null;
  if (!Array.isArray(payload?.images)) return [];
  return payload.images.flatMap((candidate, index) => {
    const image = candidate as NativeClipboardImagePayload | null;
    if (
      typeof image?.data !== "string" || image.data.length === 0 ||
      typeof image.mimeType !== "string" ||
      !image.mimeType.startsWith("image/")
    ) return [];
    try {
      const decoded = atob(image.data);
      const bytes = Uint8Array.from(decoded, (character) =>
        character.charCodeAt(0)
      );
      const name = typeof image.name === "string" && image.name.trim() !== ""
        ? image.name
        : `pasted-image-${String(index + 1)}.png`;
      return [new File([bytes], name, { type: image.mimeType })];
    } catch {
      return [];
    }
  });
}

/** Read image bytes only after the user taps the dedicated paste action. */
export async function readNativeClipboardImages(): Promise<File[]> {
  const bridge = nativeClipboardWindow().__cowboyReadClipboardImages;
  if (typeof bridge !== "function") return [];
  try {
    return nativeClipboardImageFiles(await bridge());
  } catch {
    return [];
  }
}
