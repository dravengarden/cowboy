import { useEffect, useState } from "react";
import { ImageLightbox, type ImageLightboxProps } from "@cowboy/app-shell";
import { isProtectedImageSource } from "./protectedImage";

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
    let objectUrl: string | undefined;
    void fetch(source, { credentials: "same-origin", signal: abort.signal }).then(async (response) => {
      if (!response.ok) throw new Error("Image access failed");
      const blob = await response.blob();
      if (abort.signal.aborted) return;
      objectUrl = URL.createObjectURL(blob);
      setLoaded({ source, url: objectUrl });
    }).catch(() => {});
    return () => {
      abort.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [source]);
  const images = props.images.map((item) => item.kind !== "inline-svg" && isProtectedImageSource(item.src)
    ? { ...item, src: loaded?.source === item.src && source === item.src ? loaded.url : EMPTY }
    : item);
  return <ImageLightbox {...props} images={images} />;
}
