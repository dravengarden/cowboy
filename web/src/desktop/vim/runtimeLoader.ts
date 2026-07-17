type VimModule = typeof import("./imeAutoInsertVim");
type VimRuntime = ReturnType<VimModule["createImeAutoInsertVim"]>;

let loadedModule: VimModule | null = null;
let loadPromise: Promise<VimModule> | null = null;

/**
 * Resolve the Desktop Vim chunk before an editor becomes interactive.
 *
 * CodeMirror reconfigures a live EditorState when its extension array changes.
 * That is unsafe while macOS owns native IME marked text, so callers must gate
 * the first interactive editor mount on this promise instead of adding Vim to
 * an already-editable editor.
 */
export async function preloadDesktopVimRuntime(): Promise<boolean> {
  if (loadedModule) return true;
  loadPromise ??= import("./imeAutoInsertVim");
  try {
    loadedModule = await loadPromise;
    return true;
  } catch {
    // A failed lazy chunk must not permanently disable the composer. Retrying
    // is allowed on the next mount (for example after a service-worker reload).
    loadPromise = null;
    return false;
  }
}

export function isDesktopVimRuntimeLoaded(): boolean {
  return loadedModule !== null;
}

export function createLoadedDesktopVimRuntime(): VimRuntime | null {
  return loadedModule?.createImeAutoInsertVim() ?? null;
}
