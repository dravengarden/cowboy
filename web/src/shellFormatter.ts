interface ShellFormatResult {
  ok: boolean;
  text: string;
  flatText?: string;
  context: string;
  frames?: ShellFrame[];
  summary?: string;
  error: string;
}

interface GoRuntime {
  importObject: WebAssembly.Imports;
  run(instance: WebAssembly.Instance): Promise<void>;
}

declare global {
  interface Window {
    Go?: new () => GoRuntime;
    cowboyFormatShell?: (source: string, columns: number) => ShellFormatResult;
  }
}

let runtimePromise: Promise<(source: string, columns: number) => ShellFormatResult> | undefined;
export interface ShellDisplay {
  text: string;
  flatText: string;
  context: string;
  frames: ShellFrame[];
  summary: string;
}

export interface ShellFrame {
  launcher: string;
  text: string;
  language?: string;
  label?: string;
  dialect?: "postgresql" | "sql";
  depth?: number;
  marker?: string;
  color?: number;
}

// Shell presentation is an information-density feature, not a source
// rewriter. Its primary measure is how quickly a human can recover command
// structure, executables, paths, endpoints, and embedded languages from a
// small viewport. Every projection below must therefore be bounded, visually
// quiet, and reversible by switching to Source; copy actions always retain the
// exact ACP bytes.

const NIX32 = "[0123456789abcdfghijklmnpqrsvwxyz]{32}";
const NIX_STORE_EXECUTABLE = new RegExp(
  String.raw`/nix/store/${NIX32}-[^\s'"\\]+/(?:bin|sbin|libexec)/(?:[^\s'"\\]+/)*([^/\s'"\\]+)`,
  "gu",
);

/** Collapse only executable paths whose store object matches Nix's canonical
 * 32-character Nix32 digest. `Nix bin` is a display namespace rather than a
 * decorative badge: it says where the executable came from while leaving the
 * actual command as the strongest token. Arbitrary store data paths remain
 * untouched. */
export function compactNixStoreExecutables(source: string): string {
  return source.replaceAll(NIX_STORE_EXECUTABLE, (_path, executable: string) => `Nix bin › ${executable}`);
}

/** Add display-only soft opportunities at path separators. The copy action and
 * Source mode retain the original bytes; this merely makes long assignment
 * values and URLs prefer `/` over an arbitrary mid-token phone wrap. */
export function addShellPathBreaks(source: string): string {
  return source.replaceAll(/\/(?=[^/\s])/gu, "/\u200b");
}

/** Remove the display-only marker reference that links a launcher to its
 * extracted child frame. The original command remains available in Source and
 * copy; keeping the quoted emoji in highlighted Bash makes structural chrome
 * look like executable string data. */
export function stripStructuralMarkerReference(source: string, marker?: string): string {
  if (!marker) return source;
  const escaped = marker.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
  const shellPayload = new RegExp(`\\s*(?:\\\\\\s*)?-(?:lc|c)\\s+(["'])${escaped}\\1\\s*$`, "u");
  const quotedMarker = new RegExp(`\\s*(?:\\\\\\s*)?(["'])${escaped}\\1\\s*$`, "u");
  return source.replace(shellPayload, "").replace(quotedMarker, "").trimEnd();
}

const formatCache = new Map<string, Promise<ShellDisplay | null>>();

export async function formatEmbeddedFrame(frame: ShellFrame, columns: number): Promise<ShellFrame> {
  if (frame.language === "json") {
    try {
      return { ...frame, text: JSON.stringify(JSON.parse(frame.text), null, 2) };
    } catch {
      return frame;
    }
  }
  if (frame.language === "regex") {
    return { ...frame, text: addRegexSoftBreaks(frame.text) };
  }
  if (frame.language === "jq") {
    try {
      // The parser/formatter is loaded only after a complex jq filter has been
      // extracted from Bash. Its canonical output proves the entire program
      // parsed; reflowJq then adds width-aware display breaks without touching
      // quoted operators. Failure keeps the exact decoded filter.
      const [{ format }, { reflowJq }] = await Promise.all([
        import("@jq-tools/jq"),
        import("./jqFormatter.ts"),
      ]);
      return { ...frame, text: reflowJq(format(frame.text).trimEnd(), columns) };
    } catch {
      return frame;
    }
  }
  if (frame.language === "javascript" || frame.language === "typescript") {
    const { formatEmbeddedSource } = await import("./embeddedFormatter.ts");
    return {
      ...frame,
      text: await formatEmbeddedSource({ language: frame.language, source: frame.text, columns }),
    };
  }
  if (frame.language !== "sql") return frame;
  try {
    // Kept behind the already-lazy shell formatter path: ordinary transcript
    // rendering never downloads the SQL formatter. The AST-decoded SQL is a
    // display copy only; failure preserves the exact decoded payload.
    const { format } = await import("sql-formatter");
    return {
      ...frame,
      text: format(frame.text, {
        language: frame.dialect === "postgresql" ? "postgresql" : "sql",
        // SQL keywords form the visual structure of a query. Uppercasing only
        // those tokens makes nested SQL scan quickly while identifiers,
        // functions, literals, Source mode, and copied bytes stay untouched.
        keywordCase: "upper",
        dataTypeCase: "preserve",
        functionCase: "preserve",
        logicalOperatorNewline: "before",
        expressionWidth: Math.max(36, columns - 8),
        linesBetweenQueries: 1,
      }).trimEnd(),
    };
  } catch {
    return frame;
  }
}

/** Add display-only opportunities after top-level alternations. Regex spacing
 * is dialect-sensitive, so unlike a source formatter this never inserts a
 * visible character and never changes escaped pipes or character classes. */
export function addRegexSoftBreaks(source: string): string {
  let escaped = false;
  let inClass = false;
  let result = "";
  for (const char of source) {
    result += char;
    if (escaped) {
      escaped = false;
      continue;
    }
    if (char === "\\") {
      escaped = true;
      continue;
    }
    if (char === "[") inClass = true;
    else if (char === "]") inClass = false;
    else if (char === "|" && !inClass) result += "\u200b";
  }
  return result;
}

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

function runtime(): Promise<(source: string, columns: number) => ShellFormatResult> {
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
export function formatShellForDisplay(source: string, columns = 80): Promise<ShellDisplay | null> {
  if (source.length > 64 * 1024) return Promise.resolve(null);
  const cacheKey = `${columns}\u0000${source}`;
  const cached = formatCache.get(cacheKey);
  if (cached) return cached;
  const pending = runtime()
    .then(async (format) => {
      const result = format(source, columns);
      if (!result.ok || !result.text.trim()) return null;
      const frames = await Promise.all(
        (result.frames ?? [{ launcher: result.context, text: result.text }])
          .map((frame) => formatEmbeddedFrame({ ...frame, text: frame.text.trimEnd() }, columns)),
      );
      return {
        text: compactNixStoreExecutables(result.text.trimEnd()),
        flatText: compactNixStoreExecutables((result.flatText ?? result.text).trimEnd()),
        context: result.context,
        frames: frames.map((frame) => ({
          ...frame,
          launcher: compactNixStoreExecutables(frame.launcher),
          text: compactNixStoreExecutables(frame.text),
        })),
        summary: result.summary ?? "",
      };
    })
    .catch(() => null);
  formatCache.set(cacheKey, pending);
  return pending;
}
