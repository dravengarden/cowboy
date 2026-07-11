// Route rendered markdown links through the native Tauri opener when present;
// the browser/PWA path keeps normal window.open semantics. This adapter stays
// outside mdlive so the vendored engine remains framework-agnostic.
type TauriGlobal = {
  opener?: { openUrl?: (url: string) => Promise<void> };
  core?: { invoke?: (command: string, args: Record<string, unknown>) => Promise<unknown> };
};

export function openExternalUrl(url: string): void {
  const resolved = new URL(url, window.location.href).href;
  const tauri = (globalThis as { __TAURI__?: TauriGlobal }).__TAURI__;

  if (tauri?.opener?.openUrl) {
    void tauri.opener.openUrl(resolved).catch(() => openInBrowser(resolved));
    return;
  }
  if (tauri?.core?.invoke) {
    void tauri.core.invoke("plugin:opener|open_url", { path: resolved })
      .catch(() => openInBrowser(resolved));
    return;
  }
  openInBrowser(resolved);
}

function openInBrowser(url: string): void {
  window.open(url, "_blank", "noopener,noreferrer");
}
