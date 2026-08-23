import { useSyncExternalStore } from "react";
import { type GalleryImage, ImageLightbox } from "@cowboy/app-shell";
import type { Attachment } from "./attachments";

// Fullscreen preview for a staged/queued attachment. Thin wrapper over the SHARED
// app-shell ImageLightbox (zoom / pan / swipe-to-dismiss / arrow nav) so the
// composer's attachment preview behaves exactly like a message image — one common
// image-preview component, not a cowboy-special overlay.
//
// A single module-level store drives ONE lightbox mounted at the app root, opened
// from any thumbnail (staged strip, parked draft, queued row) via
// `openLightbox(items, clickedIndex)`. Keeping it global avoids prop-drilling a
// preview callback through PendingPanel → PendingRow → the chips.
//
// Image-only (the shared component is): non-image resources (the rare attached
// file) are filtered out — the gallery is just the images, and the click index is
// remapped into it. `plate={false}`: attachments are screenshots/photos, so no
// white framing behind them.

type LightboxState = { images: GalleryImage[]; index: number } | null;

let current: LightboxState = null;
const listeners = new Set<() => void>();
const emit = (): void => {
  for (const l of listeners) l();
};

/** Open the preview on the clicked attachment (mapped into the images-only
 *  gallery). No-op when there are no previewable images in the set. */
export function openLightbox(items: Attachment[], clickedIndex: number): void {
  const imageItems = items.filter((a) => a.isImage && a.previewUrl);
  if (imageItems.length === 0) return;
  const images: GalleryImage[] = imageItems.map((a) => ({
    src: a.previewUrl as string,
    alt: a.name,
  }));
  // Remap the clicked position (an index into the full `items`) onto the
  // images-only gallery; fall back to the first image if the click was a
  // non-image (filtered out).
  const clicked = items[clickedIndex];
  const found = clicked ? imageItems.indexOf(clicked) : -1;
  current = { images, index: found >= 0 ? found : 0 };
  emit();
}

function close(): void {
  current = null;
  emit();
}

function setIndex(index: number): void {
  if (!current) return;
  current = { images: current.images, index };
  emit();
}

/** Mount once at the app root. */
export function ResourceLightbox(): React.JSX.Element | null {
  const state = useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
    () => current,
    () => null,
  );
  return (
    <ImageLightbox
      images={state?.images ?? []}
      index={state ? state.index : null}
      onIndex={setIndex}
      onClose={close}
      plate={false}
      controlsBottom="max(calc(env(safe-area-inset-bottom, 0px) + 56px), 72px)"
    />
  );
}
