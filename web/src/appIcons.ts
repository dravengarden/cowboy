import catalog from "./appIconCatalog.json" with { type: "json" };
import styles from "./appIconStyles.json" with { type: "json" };

export interface AppIcon {
  id: string;
  number: number;
  title: string;
  collection: string;
  crown: string;
  brim: string;
  background: string;
  family: string;
  tone: string;
}

export const APP_ICON_GROUPS = styles.groups;
export const DEFAULT_APP_ICON = styles.default;
export const APP_ICON_STORAGE_KEY = "cowboy-app-icon-v2";
export const APP_ICON_CHANGED = "cowboy:app-icon-changed";
const byId = new Map<string, AppIcon>(catalog.map((icon) => [icon.id, icon]));
export const APP_ICONS: readonly AppIcon[] = styles.groups.flatMap((group) =>
  group.styles.map((style) => byId.get(style.id)!)
);
const curatedIds = new Set(APP_ICONS.map((icon) => icon.id));

export function appearanceStyle(id: string) {
  const selected = appIcon(id);
  return styles.groups.flatMap((group) => group.styles).find((style) =>
    style.id === selected.id
  ) ?? {
    id: selected.id,
    name: selected.title,
    themeColor: selected.crown,
  };
}

export function subscribeAppIcon(listener: () => void): () => void {
  globalThis.addEventListener(APP_ICON_CHANGED, listener);
  return () => globalThis.removeEventListener(APP_ICON_CHANGED, listener);
}

export function resolveIconPreference(
  stored: string | null,
  legacy: string | null,
): string {
  // Version 1 stored the old primary icon even when no custom choice was made.
  // Keep other old selections; subsequent defaults use an explicit sentinel.
  if (stored !== null) return appIcon(stored === "default" ? null : stored).id;
  return appIcon(legacy === "palette-054" ? null : legacy).id;
}

export function appIcon(id: string | null | undefined): AppIcon {
  return byId.get(id ?? "") ?? byId.get(DEFAULT_APP_ICON)!;
}

export function appIconAsset(
  id: string,
  size: 96 | 180 | 192 | 512 = 192,
): string {
  return `/app-icons/v5/${appIcon(id).id}/icon-${size}.png`;
}

// Native Neon supplies paired appearances; archived styles retain their artwork.
export function appIconAppearanceAsset(id: string, dark: boolean): string {
  return appIcon(id).id === "palette-103" && !dark
    ? "/app-icons/v6/palette-103/icon-light-192.png"
    : appIconAsset(id, 192);
}

export function appIconTabAsset(id: string): string {
  const icon = appIcon(id);
  return icon.collection === "palette"
    ? `/app-icons/v7/${icon.id}/favicon.svg`
    : appIconAsset(icon.id, 192);
}

export function appIconInstallPath(id: string): string {
  return `/app-icons/v5/${appIcon(id).id}/install.html`;
}

export function iconColorFamily(hex: string): string {
  const [r = 0, g = 0, b = 0] =
    hex.slice(1).match(/../g)?.map((v) => parseInt(v, 16) / 255) ?? [];
  const max = Math.max(r, g, b), min = Math.min(r, g, b), delta = max - min;
  if (max === 0 || delta / max < 0.18) return "neutral";
  let hue = max === r
    ? (g - b) / delta
    : max === g
    ? (b - r) / delta + 2
    : (r - g) / delta + 4;
  hue = ((hue * 60) + 360) % 360;
  if (hue < 20 || hue >= 345) return "red";
  if (hue < 48) return "orange";
  if (hue < 75) return "gold";
  if (hue < 255) return "blue";
  if (hue < 290) return "purple";
  return "pink";
}

export function filterAppIcons(input: {
  query?: string;
  family?: string;
  tone?: string;
}): AppIcon[] {
  const query = (input.query ?? "").trim().toLowerCase();
  return APP_ICONS.filter((icon) =>
    (!input.family || input.family === "all" ||
      [iconColorFamily(icon.crown), iconColorFamily(icon.brim)].includes(
        input.family,
      )) &&
    (!input.tone || input.tone === "all" || icon.tone === input.tone) &&
    (!query || (/^\d+$/.test(query)
      ? icon.number === Number(query)
      : `${icon.number} ${icon.title} ${icon.id} ${icon.crown} ${icon.brim} ${icon.background}`
        .toLowerCase().includes(query)))
  );
}

export interface NativeAppIconState {
  supported: boolean;
  current: string;
  available: string[];
}

type IconWindow = typeof globalThis & {
  __cowboyNativeShell?: boolean;
  __TAURI_INTERNALS__?: unknown;
  __cowboyAppIcon?: (
    request: { action: "state" | "set"; id?: string },
  ) => Promise<unknown>;
};

export function isNativeIconSurface(): boolean {
  const root = globalThis as IconWindow;
  return root.__cowboyNativeShell === true || !!root.__TAURI_INTERNALS__;
}

export function parseNativeAppIconState(raw: unknown): NativeAppIconState {
  if (!raw || typeof raw !== "object") {
    throw new Error("Invalid native icon response.");
  }
  const value = raw as Record<string, unknown>;
  if (value.ok !== true) {
    throw new Error(
      typeof value.error === "string"
        ? value.error
        : "The system could not change the icon.",
    );
  }
  if (
    typeof value.supported !== "boolean" || typeof value.current !== "string" ||
    !Array.isArray(value.available) ||
    !value.available.every((id) => typeof id === "string")
  ) {
    throw new Error("Invalid native icon response.");
  }
  if (!byId.has(value.current)) {
    throw new Error(
      "Refresh Cowboy to recognize the native app's current icon.",
    );
  }
  return {
    supported: value.supported,
    current: appIcon(value.current).id,
    available: value.available.filter((id): id is string =>
      typeof id === "string" && byId.has(id)
    ),
  };
}

export async function nativeAppIconState(): Promise<NativeAppIconState | null> {
  const bridge = (globalThis as IconWindow).__cowboyAppIcon;
  return typeof bridge === "function"
    ? parseNativeAppIconState(await bridge({ action: "state" }))
    : null;
}

function readPreference(): string {
  try {
    return resolveIconPreference(
      globalThis.localStorage?.getItem(APP_ICON_STORAGE_KEY) ?? null,
      globalThis.localStorage?.getItem("cowboy-app-icon-v1") ?? null,
    );
  } catch {
    return DEFAULT_APP_ICON;
  }
}

let current = readPreference();
export function currentAppIcon(): string {
  return current;
}

export function applyAppIconDocument(id: string): void {
  const doc = globalThis.document;
  if (!doc) return;
  const selected = appIcon(id);
  for (
    const link of doc.querySelectorAll<HTMLLinkElement>(
      'link[rel="icon"], link[rel="alternate icon"]',
    )
  ) {
    link.remove();
  }
  const favicon = doc.createElement("link");
  favicon.rel = "icon";
  favicon.href = appIconTabAsset(selected.id);
  favicon.type = favicon.href.endsWith(".svg") ? "image/svg+xml" : "image/png";
  favicon.sizes.value = favicon.type === "image/svg+xml" ? "any" : "192x192";
  doc.head.appendChild(favicon);
  for (
    const link of doc.querySelectorAll<HTMLLinkElement>(
      'link[rel="apple-touch-icon"], link[rel="apple-touch-icon-precomposed"]',
    )
  ) {
    link.href = appIconAsset(selected.id, 180);
  }
  const manifest = doc.querySelector<HTMLLinkElement>('link[rel="manifest"]');
  if (manifest) {
    manifest.href = `/app-icons/v5/${selected.id}/manifest.webmanifest`;
  }
}

function commitPreference(id: string): void {
  current = appIcon(id).id;
  try {
    globalThis.localStorage?.setItem(
      APP_ICON_STORAGE_KEY,
      current === DEFAULT_APP_ICON ? "default" : current,
    );
  } catch { /* Window-only preference. */ }
  applyAppIconDocument(current);
  globalThis.dispatchEvent?.(new Event(APP_ICON_CHANGED));
}

export async function selectAppIcon(
  id: string,
  options: { themeOnly?: boolean } = {},
): Promise<void> {
  if (!curatedIds.has(id)) throw new Error("Choose one of the curated styles.");
  if (isNativeIconSurface() && !options.themeOnly) {
    const bridge = (globalThis as IconWindow).__cowboyAppIcon;
    if (typeof bridge !== "function") {
      throw new Error(
        "This native app does not support automatic icon changes. Download the icon to use with your system's icon controls, or update the iOS app.",
      );
    }
    const state = parseNativeAppIconState(await bridge({ action: "set", id }));
    if (!state.supported || state.current !== id) {
      throw new Error("The system did not apply this icon.");
    }
  }
  commitPreference(id);
}

export function initializeAppIcons(): void {
  const url = new URL(globalThis.location.href);
  const fromLink = url.searchParams.get("app-icon");
  let consumed: string | null = null;
  try {
    consumed =
      globalThis.localStorage?.getItem("cowboy-icon-install-handoff") ?? null;
  } catch { /* Optional storage. */ }
  if (
    fromLink && byId.has(fromLink) && fromLink !== consumed &&
    !isNativeIconSurface()
  ) {
    commitPreference(fromLink);
    try {
      globalThis.localStorage?.setItem("cowboy-icon-install-handoff", fromLink);
    } catch { /* Optional storage. */ }
  } else applyAppIconDocument(current);
  // Consume the installation handoff once. A stale start_url must not undo a
  // later selection in the same installed browser's local storage.
  if (fromLink) {
    url.searchParams.delete("app-icon");
    globalThis.history.replaceState(globalThis.history.state, "", url);
  }
  const syncNative = (): void => {
    void nativeAppIconState().then((state) => {
      if (state?.supported) commitPreference(state.current);
    }).catch(() => undefined);
  };
  syncNative();
  globalThis.addEventListener("cowboy:native-resume", syncNative);
  globalThis.addEventListener("storage", (event: StorageEvent) => {
    if (event.key === APP_ICON_STORAGE_KEY || event.key === null) {
      current = readPreference();
      applyAppIconDocument(current);
      globalThis.dispatchEvent(new Event(APP_ICON_CHANGED));
    }
  });
}
