import { forwardRef, useEffect, useImperativeHandle, useRef, type ImgHTMLAttributes } from "react";
import { isProtectedImageSource, setProtectedImageSource } from "./protectedImage";

/** Keeps the same img DOM, layout, load events and gesture ref as the host. */
export const ProtectedImage = forwardRef<HTMLImageElement, ImgHTMLAttributes<HTMLImageElement>>(
  function ProtectedImage({ src, ...props }, forwardedRef) {
    const element = useRef<HTMLImageElement>(null);
    useImperativeHandle(forwardedRef, () => element.current!, []);
    useEffect(() => {
      const image = element.current;
      if (image && src) return setProtectedImageSource(image, src);
      return undefined;
    }, [src]);
    return <img {...props} ref={element} src={src && !isProtectedImageSource(src) ? src : undefined} />;
  },
);
