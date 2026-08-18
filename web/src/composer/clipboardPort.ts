// Product clipboard port. The composer dock never asks "are we native?".
// It asks a port: can I offer Paste, and what do I get when the user taps?
//
//   native  — WKWebView pasteboard bridges (status without reading payload)
//   web     — navigator.clipboard after a user gesture (Safari / PWA)
//
// Status on web cannot see the pasteboard. Offering Paste means the API
// exists; emptiness is discovered on the tap.

import {
  nativeClipboardImageStatus,
  nativeClipboardPasteAvailable,
  readNativeClipboardImageOutcome,
  readNativeClipboardText,
  supportsNativeClipboardImages,
} from "../nativeShell";
import { readWebClipboard, supportsWebClipboardRead } from "./webClipboard";

export type ClipboardSurface = "native" | "web";

export interface ClipboardAvailability {
  surface: ClipboardSurface;
  pasteAvailable: boolean;
  /** True when the next tap should stage image placeholders first. */
  stageImagesFirst: boolean;
  imageCount: number;
}

export interface ClipboardContents {
  text: string;
  files: File[];
}

export interface ClipboardPort {
  readonly surface: ClipboardSurface;
  status(): Promise<ClipboardAvailability>;
  read(): Promise<ClipboardContents>;
}

export function hasNativeClipboardBridge(
  root: {
    __cowboyReadClipboard?: unknown;
    __cowboyClipboardImageStatus?: unknown;
    __cowboyReadClipboardImages?: unknown;
  } = globalThis as typeof globalThis & {
    __cowboyReadClipboard?: unknown;
    __cowboyClipboardImageStatus?: unknown;
    __cowboyReadClipboardImages?: unknown;
  },
): boolean {
  return typeof root.__cowboyReadClipboard === "function" ||
    (typeof root.__cowboyClipboardImageStatus === "function" &&
      typeof root.__cowboyReadClipboardImages === "function");
}

export function createClipboardPort(): ClipboardPort {
  return hasNativeClipboardBridge() ? nativeClipboardPort() : webClipboardPort();
}

export function nativeClipboardPort(): ClipboardPort {
  return {
    surface: "native",
    async status() {
      const status = await nativeClipboardImageStatus();
      return {
        surface: "native",
        pasteAvailable: nativeClipboardPasteAvailable(status),
        stageImagesFirst: status.hasImages,
        imageCount: status.imageCount,
      };
    },
    async read() {
      if (supportsNativeClipboardImages()) {
        const images = await readNativeClipboardImageOutcome();
        if (images.files.length > 0) {
          return { text: "", files: images.files };
        }
      }
      return { text: await readNativeClipboardText(), files: [] };
    },
  };
}

export function webClipboardPort(): ClipboardPort {
  return {
    surface: "web",
    async status() {
      const available = supportsWebClipboardRead();
      return {
        surface: "web",
        pasteAvailable: available,
        stageImagesFirst: false,
        imageCount: 0,
      };
    },
    read: () => readWebClipboard(),
  };
}
