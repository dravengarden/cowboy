import { useContext, useEffect, useState } from "react";
import { ImageLightbox, type ImageLightboxProps } from "@cowboy/app-shell";
import {
  imageDisplayUrl,
  type ImageDisplayUrl,
  isProtectedImageSource,
  remoteImageHost,
} from "./protectedImage";
import { SurfaceContext } from "./surface/SurfaceProfile";

const EMPTY = "data:image/gif;base64,R0lGODlhAQABAAD/ACwAAAAAAQABAAACADs=";

/** Cowboy owns authenticated loading; the shared gallery owns all gestures. */
export function ProtectedImageLightbox(props: ImageLightboxProps): React.JSX.Element | null {
  const current = props.index === null ? undefined : props.images[props.index];
  const source = current && current.kind !== "inline-svg" && isProtectedImageSource(current.src)
    ? current.src : undefined;
  const [loaded, setLoaded] = useState<{ source: string; url: string }>();
  useEffect(() => {
    if (!source) return;
    const abort = new AbortController();
    let display: ImageDisplayUrl | undefined;
    void fetch(source, { credentials: "same-origin", signal: abort.signal }).then(async (response) => {
      if (!response.ok) throw new Error("Image access failed");
      const blob = await response.blob();
      if (abort.signal.aborted) return;
      const prepared = await imageDisplayUrl(blob);
      if (abort.signal.aborted) {
        prepared.release();
        return;
      }
      display = prepared;
      setLoaded({ source, url: prepared.url });
    }).catch(() => {});
    return () => {
      abort.abort();
      display?.release();
    };
  }, [source]);
  // The gallery pages and preloads. A remote neighbour stays empty rather than
  // trip the CSP.
  const images = props.images.map((item) => {
    if (item.kind === "inline-svg") return item;
    if (isProtectedImageSource(item.src)) {
      return { ...item, src: loaded?.source === item.src && source === item.src ? loaded.url : EMPTY };
    }
    return remoteImageHost(item.src) === null ? item : { ...item, src: EMPTY };
  });
  // Desktop is keyboard-first: show the lightbox key map there.
  const desktop = useContext(SurfaceContext)?.kind === "desktop";
  return <ImageLightbox keyHints={desktop} {...props} images={images} />;
}
