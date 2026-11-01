import { Box } from "@mui/material";
import { forwardRef, useEffect, useImperativeHandle, useRef, type ImgHTMLAttributes } from "react";
import { openExternalUrl, safeAuthenticationUrl } from "./openExternal";
import { isProtectedImageSource, remoteImageHost, setProtectedImageSource } from "./protectedImage";

/** The document CSP refuses remote images, so this names the host a click would
 * contact and opens the image outside the app. */
function RemoteImagePlaceholder(
  { source, alt, className }: { source: string; alt: string | undefined; className: string | undefined },
): React.JSX.Element {
  const host = remoteImageHost(source);
  const target = safeAuthenticationUrl(source);
  return (
    <Box
      component="button"
      type="button"
      className={className}
      data-remote-image-placeholder={host ?? ""}
      disabled={!target}
      title={source}
      onClick={(event: React.MouseEvent): void => {
        event.stopPropagation();
        if (target) openExternalUrl(target);
      }}
      sx={{
        display: "inline-block",
        px: 1,
        py: 0.5,
        font: "inherit",
        fontSize: "0.85em",
        color: "text.secondary",
        bgcolor: "action.hover",
        border: 1,
        borderColor: "divider",
        borderRadius: 1,
        overflowWrap: "anywhere",
        textAlign: "start",
        cursor: target ? "pointer" : "default",
      }}
    >
      {target ? `Open image from ${host}` : "Image address is not valid"}
      {alt ? ` · ${alt}` : ""}
    </Box>
  );
}

/** Keeps the same img DOM, layout, load events and gesture ref as the host. */
export const ProtectedImage = forwardRef<HTMLImageElement, ImgHTMLAttributes<HTMLImageElement>>(
  function ProtectedImage({ src, ...props }, forwardedRef) {
    const element = useRef<HTMLImageElement>(null);
    useImperativeHandle(forwardedRef, () => element.current!, []);
    const remote = src !== undefined && remoteImageHost(src) !== null;
    useEffect(() => {
      const image = element.current;
      if (image && src && !remote) return setProtectedImageSource(image, src);
      return undefined;
    }, [src, remote]);
    if (src && remote) {
      return <RemoteImagePlaceholder source={src} alt={props.alt} className={props.className} />;
    }
    return <img {...props} ref={element} src={src && !isProtectedImageSource(src) ? src : undefined} />;
  },
);
