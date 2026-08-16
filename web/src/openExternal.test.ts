import {
  closeAuthenticationBrowser,
  hasNativeAuthenticationBrowser,
  hasNativeExternalOpener,
  openAuthenticationUrl,
  openExternalUrl,
  safeAuthenticationUrl,
  safeExternalUrl,
  shouldRouteAuthenticationClick,
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

Deno.test("Provider authentication accepts web URLs only", () => {
  if (safeAuthenticationUrl("https://example.com/login") === null) {
    throw new Error("expected HTTPS authentication URL");
  }
  for (const url of ["mailto:user@example.com", "tel:+15551234567"]) {
    if (safeAuthenticationUrl(url) !== null) {
      throw new Error(`expected rejected authentication URL: ${url}`);
    }
  }
});

Deno.test("Provider authentication prefers and closes the native Safari sheet", () => {
  let opened = "";
  let closes = 0;
  const root = globalThis as typeof globalThis & {
    __cowboyOpenAuthenticationBrowser?: (url: string) => boolean;
    __cowboyCloseAuthenticationBrowser?: () => void;
  };
  root.__cowboyOpenAuthenticationBrowser = (url) => {
    opened = url;
    return true;
  };
  root.__cowboyCloseAuthenticationBrowser = () => closes += 1;
  try {
    openAuthenticationUrl("https://example.com/authorize?code=123");
    closeAuthenticationBrowser();
    if (opened !== "https://example.com/authorize?code=123") {
      throw new Error(`unexpected native authentication URL: ${opened}`);
    }
    if (closes !== 1) throw new Error(`expected one native close, got ${closes}`);
  } finally {
    delete root.__cowboyOpenAuthenticationBrowser;
    delete root.__cowboyCloseAuthenticationBrowser;
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

Deno.test("authentication links route through an iOS sheet when available", () => {
  const root = globalThis as typeof globalThis & {
    __cowboyOpenAuthenticationBrowser?: (url: string) => boolean;
  };
  const click = {
    button: 0,
    defaultPrevented: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
  };
  if (shouldRouteAuthenticationClick(click)) {
    throw new Error("browser authentication anchor must retain target=_blank");
  }
  root.__cowboyOpenAuthenticationBrowser = () => true;
  try {
    if (!hasNativeAuthenticationBrowser()) {
      throw new Error("expected native authentication browser detection");
    }
    if (!shouldRouteAuthenticationClick(click)) {
      throw new Error("native authentication must use the Safari sheet");
    }
    if (shouldRouteAuthenticationClick({ ...click, shiftKey: true })) {
      throw new Error("modified authentication clicks retain anchor semantics");
    }
  } finally {
    delete root.__cowboyOpenAuthenticationBrowser;
  }
  if (hasNativeAuthenticationBrowser()) {
    throw new Error("native authentication browser must clear with the bridge");
  }
});
