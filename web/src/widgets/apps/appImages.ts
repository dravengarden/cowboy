// Images an app draws (docs/widget-apps.md §6). Every image of the node is
// loaded into a local blob before the app runs, so which images the app
// draws, and when, never causes a request: drawing is only a lookup by key.
// The Controller verified each source as a still PNG within the app bounds;
// this host still loads only same-origin artifacts and inline PNGs.

import { createContext, useContext, useEffect, useRef, useState } from "react";
import type { ImageSource } from "../../../../contracts/widget-v1.ts";
import { pictureUrl } from "../WidgetImages";

export type AppImages = ReadonlyMap<string, string>;

export const AppImagesContext = createContext<AppImages>(new Map());

/** The local URL of the app image `key`, or null (the app shows `alt`). */
export function useAppImage(key: string): string | null {
  return useContext(AppImagesContext).get(key) ?? null;
}

async function load(source: ImageSource): Promise<Blob | null> {
  const url = pictureUrl(source, true);
  if (!url || (source.kind === "artifact" && !url.endsWith(".png"))) return null;
  const response = await fetch(url, { credentials: "same-origin" });
  if (!response.ok) return null;
  const blob = await response.blob();
  return blob.size > 0 ? new Blob([blob], { type: "image/png" }) : null;
}

/** Loads every image of an app; null until the first set has settled. An
 *  image that fails to load stays absent, so its `Image` shows the alt text.
 *  When the images change, the previous set stays drawn until the new one is
 *  ready, then its blobs are released. */
export function useAppImagesLoaded(
  images: Readonly<Record<string, ImageSource>> | undefined,
): AppImages | null {
  const signature = JSON.stringify(images ?? {});
  const [loaded, setLoaded] = useState<AppImages | null>(null);
  const current = useRef<string[]>([]);
  useEffect(() => {
    const entries = Object.entries(JSON.parse(signature) as Record<string, ImageSource>);
    let alive = true;
    void Promise.all(
      entries.map(async ([key, source]) => [key, await load(source).catch(() => null)] as const),
    ).then((results) => {
      if (!alive) return;
      const map = new Map<string, string>();
      const urls: string[] = [];
      for (const [key, blob] of results) {
        if (!blob) continue;
        const url = URL.createObjectURL(blob);
        urls.push(url);
        map.set(key, url);
      }
      const previous = current.current;
      current.current = urls;
      setLoaded(map);
      // Released after the new set is drawn.
      setTimeout(() => {
        for (const url of previous) URL.revokeObjectURL(url);
      }, 1000);
    });
    return () => {
      alive = false;
    };
  }, [signature]);
  useEffect(
    () => () => {
      for (const url of current.current) URL.revokeObjectURL(url);
    },
    [],
  );
  return loaded;
}
