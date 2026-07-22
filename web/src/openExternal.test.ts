import {
  hasNativeExternalOpener,
  openExternalUrl,
  safeExternalUrl,
  shouldRouteExternalClick,
} from "./openExternal";

Deno.test("external links allow explicit network and contact protocols", () => {
  for (const url of [
    "https://example.com/docs?q=1",
    "http://127.0.0.1:4160/health",
    "mailto:reader@example.com",
    "tel:+15551234567",
  ]) {
    if (safeExternalUrl(url) !== new URL(url).href) {
      throw new Error(`expected allowed external URL: ${url}`);
    }
  }
});

Deno.test("external links reject executable and local protocols", () => {
  for (const url of [
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "file:///etc/passwd",
    "custom-handler:payload",
  ]) {
    if (safeExternalUrl(url) !== null) {
      throw new Error(`expected rejected external URL: ${url}`);
    }
  }
});

Deno.test("external links reject relative and malformed values", () => {
  for (const url of ["/relative", "notes/chapter-1", "not a URL", "http://["]) {
    if (safeExternalUrl(url) !== null) {
      throw new Error(`expected invalid external URL: ${url}`);
    }
  }
});

Deno.test("Tauri fallback passes open_url its url argument", () => {
  let command = "";
  let args: Record<string, unknown> = {};
  const root = globalThis as typeof globalThis & {
    __TAURI__?: {
      core: {
        invoke: (
          nextCommand: string,
          nextArgs: Record<string, unknown>,
        ) => Promise<unknown>;
      };
    };
  };
  root.__TAURI__ = {
    core: {
      invoke: (nextCommand, nextArgs) => {
        command = nextCommand;
        args = nextArgs;
        return Promise.resolve();
      },
    },
  };
  try {
    openExternalUrl("https://example.com/authorize?code=123");
    if (command !== "plugin:opener|open_url") {
      throw new Error(`unexpected Tauri command: ${command}`);
    }
    if (args.url !== "https://example.com/authorize?code=123") {
      throw new Error(`expected url argument, got ${JSON.stringify(args)}`);
    }
    if ("path" in args) throw new Error("open_url must not receive a path argument");
  } finally {
    delete root.__TAURI__;
  }
});

Deno.test("Tauri v2 internals open native-shell links with the URL argument", () => {
  let command = "";
  let args: Record<string, unknown> = {};
  const root = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: {
      invoke: (nextCommand: string, nextArgs: Record<string, unknown>) => Promise<unknown>;
    };
  };
  root.__TAURI_INTERNALS__ = {
    invoke: (nextCommand, nextArgs) => {
      command = nextCommand;
      args = nextArgs;
      return Promise.resolve();
    },
  };
  try {
    if (!hasNativeExternalOpener()) throw new Error("expected native opener detection");
    openExternalUrl("https://example.com/native");
    if (command !== "plugin:opener|open_url") throw new Error(`unexpected command: ${command}`);
    if (args.url !== "https://example.com/native") {
      throw new Error(`expected native URL argument, got ${JSON.stringify(args)}`);
    }
  } finally {
    delete root.__TAURI_INTERNALS__;
  }
});

Deno.test("only unmodified native primary clicks override anchor navigation", () => {
  const root = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: { invoke: () => Promise<unknown> };
  };
  const click = {
    button: 0,
    defaultPrevented: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
  };
  if (shouldRouteExternalClick(click)) throw new Error("browser anchor must remain native");
  root.__TAURI_INTERNALS__ = { invoke: () => Promise.resolve() };
  try {
    if (!shouldRouteExternalClick(click)) throw new Error("native primary click must use opener");
    if (shouldRouteExternalClick({ ...click, metaKey: true })) {
      throw new Error("modified clicks must retain anchor semantics");
    }
  } finally {
    delete root.__TAURI_INTERNALS__;
  }
});
