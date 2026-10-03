/** Native image elements cannot attach authentication headers. This lifetime
 * loads only protected Cowboy resources through the authenticated fetch path. */
export function isProtectedImageSource(source: string): boolean {
  const url = new URL(source, globalThis.location.href);
  return url.origin === globalThis.location.origin && url.pathname.startsWith("/api/");
}

export function setProtectedImageSource(image: HTMLImageElement, source: string): () => void {
  const url = new URL(source, globalThis.location.href);
  if (!isProtectedImageSource(source)) {
    image.src = source;
    return () => {};
  }
  const controller = new AbortController();
  let objectUrl: string | undefined;
  image.removeAttribute("src");
  void fetch(url, { credentials: "same-origin", signal: controller.signal }).then(async (response) => {
    if (!response.ok) throw new Error("Image access failed");
    const blob = await response.blob();
    if (controller.signal.aborted) return;
    objectUrl = URL.createObjectURL(blob);
    image.src = objectUrl;
  }).catch(() => {
    if (!controller.signal.aborted) image.dispatchEvent(new Event("error"));
  });
  return () => {
    controller.abort();
    image.removeAttribute("src");
    if (objectUrl) URL.revokeObjectURL(objectUrl);
  };
}
