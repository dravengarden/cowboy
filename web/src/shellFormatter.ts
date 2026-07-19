interface ShellFormatResult {
  ok: boolean;
  text: string;
  context: string;
  error: string;
}

interface GoRuntime {
  importObject: WebAssembly.Imports;
  run(instance: WebAssembly.Instance): Promise<void>;
}

declare global {
  interface Window {
    Go?: new () => GoRuntime;
    cowboyFormatShell?: (source: string) => ShellFormatResult;
  }
}

let runtimePromise: Promise<(source: string) => ShellFormatResult> | undefined;
export interface ShellDisplay {
  text: string;
  context: string;
}

const formatCache = new Map<string, Promise<ShellDisplay | null>>();

function loadScript(source: string): Promise<void> {
  const existing = document.querySelector<HTMLScriptElement>(`script[src="${source}"]`);
  if (existing?.dataset.loaded === "true") return Promise.resolve();
  return new Promise((resolve, reject) => {
    const script = existing ?? document.createElement("script");
    script.addEventListener("load", () => {
      script.dataset.loaded = "true";
      resolve();
    }, { once: true });
    script.addEventListener("error", () => reject(new Error(`Failed to load ${source}`)), { once: true });
    if (!existing) {
      script.src = source;
      script.async = true;
      document.head.append(script);
    }
  });
}

async function instantiate(go: GoRuntime): Promise<WebAssembly.Instance> {
  const response = await fetch("/shellfmt.wasm");
  if (!response.ok) throw new Error(`shell formatter HTTP ${response.status}`);
  try {
    return (await WebAssembly.instantiateStreaming(response.clone(), go.importObject)).instance;
  } catch {
    return (await WebAssembly.instantiate(await response.arrayBuffer(), go.importObject)).instance;
  }
}

function runtime(): Promise<(source: string) => ShellFormatResult> {
  runtimePromise ??= (async () => {
    await loadScript("/wasm_exec.js");
    if (!window.Go) throw new Error("Go WASM runtime unavailable");
    const go = new window.Go();
    const instance = await instantiate(go);
    void go.run(instance);
    for (let attempt = 0; attempt < 20 && !window.cowboyFormatShell; attempt += 1) {
      await new Promise((resolve) => globalThis.setTimeout(resolve, 0));
    }
    if (!window.cowboyFormatShell) throw new Error("Shell formatter did not initialize");
    return window.cowboyFormatShell;
  })();
  return runtimePromise;
}

/** Format a display-only copy with mvdan/sh. Failure is intentionally null so
 * callers preserve the exact source without surfacing parser errors as UI. */
export function formatShellForDisplay(source: string): Promise<ShellDisplay | null> {
  if (source.length > 64 * 1024) return Promise.resolve(null);
  const cached = formatCache.get(source);
  if (cached) return cached;
  const pending = runtime()
    .then((format) => {
      const result = format(source);
      return result.ok && result.text.trim()
        ? { text: result.text.trimEnd(), context: result.context }
        : null;
    })
    .catch(() => null);
  formatCache.set(source, pending);
  return pending;
}
