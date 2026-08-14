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
  return (window as { __cowboyNativeShell?: boolean }).__cowboyNativeShell ===
    true;
}

interface NativeClipboardImageStatusPayload {
  hasImages?: unknown;
  hasText?: unknown;
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

export interface NativeClipboardImageReadOutcome {
  files: File[];
  payloadCount: number;
  changeCount: number;
  state: "ok" | "empty" | "invalid_payload" | "missing_bridge" | "bridge_error";
}

interface CowboyNativeClipboardWindow extends Window {
  __cowboyReadClipboard?: () => Promise<unknown>;
  __cowboyClipboardImageStatus?: () => Promise<unknown>;
  __cowboyReadClipboardImages?: () => Promise<unknown>;
}

export interface NativeClipboardImageStatus {
  supported: boolean;
  hasImages: boolean;
  hasText: boolean;
  canReadText: boolean;
  textAvailabilityKnown: boolean;
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
export async function nativeClipboardImageStatus(): Promise<
  NativeClipboardImageStatus
> {
  const root = nativeClipboardWindow();
  const bridge = root.__cowboyClipboardImageStatus;
  const canReadText = typeof root.__cowboyReadClipboard === "function";
  if (
    typeof bridge !== "function" ||
    typeof root.__cowboyReadClipboardImages !== "function"
  ) {
    return {
      supported: false,
      hasImages: false,
      hasText: false,
      canReadText,
      textAvailabilityKnown: !canReadText,
      imageCount: 0,
      changeCount: -1,
    };
  }
  try {
    const raw = await bridge() as NativeClipboardImageStatusPayload | null;
    const textAvailabilityKnown = !canReadText ||
      typeof raw?.hasText === "boolean";
    return {
      supported: true,
      hasImages: raw?.hasImages === true,
      hasText: raw?.hasText === true && canReadText,
      canReadText,
      textAvailabilityKnown,
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
      hasText: false,
      canReadText,
      textAvailabilityKnown: !canReadText,
      imageCount: 0,
      changeCount: -1,
    };
  }
}

/**
 * Older native shells can read text but predate the metadata `hasText` field.
 * Keep their explicit Paste action usable without speculatively reading the
 * clipboard; the tap itself remains the first payload access.
 */
export function nativeClipboardPasteAvailable(
  status: NativeClipboardImageStatus,
): boolean {
  return status.hasImages || status.hasText ||
    (status.canReadText && !status.textAvailabilityKnown);
}

/** Read plain text only after the user taps the dedicated Paste action. */
export async function readNativeClipboardText(): Promise<string> {
  const bridge = nativeClipboardWindow().__cowboyReadClipboard;
  if (typeof bridge !== "function") return "";
  try {
    const value = await bridge();
    return typeof value === "string" ? value : "";
  } catch {
    return "";
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
      const bytes = Uint8Array.from(
        decoded,
        (character) => character.charCodeAt(0),
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
export async function readNativeClipboardImageOutcome(): Promise<
  NativeClipboardImageReadOutcome
> {
  const bridge = nativeClipboardWindow().__cowboyReadClipboardImages;
  if (typeof bridge !== "function") {
    return {
      files: [],
      payloadCount: 0,
      changeCount: -1,
      state: "missing_bridge",
    };
  }
  try {
    const raw = await bridge() as NativeClipboardImagesPayload | null;
    const payloadCount = Array.isArray(raw?.images) ? raw.images.length : 0;
    const changeCount = typeof raw?.changeCount === "number" &&
        Number.isSafeInteger(raw.changeCount)
      ? raw.changeCount
      : -1;
    if (!Array.isArray(raw?.images)) {
      return {
        files: [],
        payloadCount,
        changeCount,
        state: "invalid_payload",
      };
    }
    const files = nativeClipboardImageFiles(raw);
    return {
      files,
      payloadCount,
      changeCount,
      state: files.length > 0
        ? "ok"
        : payloadCount > 0
        ? "invalid_payload"
        : "empty",
    };
  } catch {
    return {
      files: [],
      payloadCount: 0,
      changeCount: -1,
      state: "bridge_error",
    };
  }
}

export async function readNativeClipboardImages(): Promise<File[]> {
  return (await readNativeClipboardImageOutcome()).files;
}
