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
