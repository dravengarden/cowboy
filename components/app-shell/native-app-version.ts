type NativeVersionHost = {
  __cowboyNativeApp?: { version?: unknown };
  __TAURI__?: {
    app?: { getVersion?: () => Promise<string> };
    core?: { invoke?: (command: string) => Promise<unknown> };
  };
};

// SideStore stamps the installed bundle after compilation. Tauri's package
// version remains the Cargo/config version, so prefer the OS bundle metadata.
export async function getNativeAppVersion(
  host: NativeVersionHost = globalThis as NativeVersionHost,
): Promise<string | null> {
  const installed = host.__cowboyNativeApp?.version;
  if (typeof installed === "string" && /^\d+\.\d+\.\d+$/.test(installed)) {
    return installed;
  }
  const api = host.__TAURI__;
  try {
    if (api?.app?.getVersion) return await api.app.getVersion();
    if (api?.core?.invoke) {
      const version = await api.core.invoke("plugin:app|version");
      return typeof version === "string" ? version : null;
    }
  } catch {
    // An unavailable native bridge must never block the Web product.
  }
  return null;
}

export type NativeAppPlatform = "ios" | "macos" | "other";

type NativePlatformHost = NativeVersionHost & {
  navigator?: { userAgent?: string; maxTouchPoints?: number };
};

/**
 * Which Apple shell runs this page. The iOS/iPadOS shell injects its
 * installed bundle metadata (`__cowboyNativeApp`, CowboyAppIconBridge, built
 * against UIKit only); the macOS Tauri shell does not. An older iOS build
 * without that bridge is still recognised by its user agent: an iPhone, or
 * an iPad that reports "Macintosh" but has a touch screen.
 */
export function getNativeAppPlatform(
  host: NativePlatformHost = globalThis as NativePlatformHost,
): NativeAppPlatform {
  if (host.__cowboyNativeApp) return "ios";
  const agent = host.navigator?.userAgent ?? "";
  if (/iPhone|iPad|iPod/.test(agent)) return "ios";
  if (/Macintosh/.test(agent)) {
    return (host.navigator?.maxTouchPoints ?? 0) > 0 ? "ios" : "macos";
  }
  return "other";
}
