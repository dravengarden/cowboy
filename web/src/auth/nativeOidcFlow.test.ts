import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import {
  nativeOidcCancelPath,
  nativeOidcEventsPath,
  nativeOidcPollPath,
  type ProductOidcProvider,
} from "./authApi.ts";
import {
  browserOidcFlowSupported,
  nativeOidcEventsUrl,
  nativeOidcStartUrl,
  runBrowserOidc,
  runNativeOidc,
} from "./nativeOidcFlow.ts";
import { newPkceBinding } from "./pkce.ts";
import { NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT } from "../openExternal.ts";

const cardea: ProductOidcProvider = {
  id: "cardea",
  display_name: "Cardea",
  button_label: "Continue with Cardea",
  start_url: "/api/auth/oidc/start",
};

Deno.test("native OIDC start exposes only independent PKCE challenges", async () => {
  const [code, handoff] = await Promise.all([
    newPkceBinding(),
    newPkceBinding(),
  ]);
  const url = new URL(
    nativeOidcStartUrl(
      "https://cowboy.example",
      cardea,
      code.challenge,
      handoff.challenge,
    ),
  );
  assertEquals(url.origin, "https://cowboy.example");
  assertEquals(url.pathname, "/api/auth/oidc/start");
  assertEquals(url.searchParams.get("client"), "browser-shell");
  assertEquals(url.searchParams.get("code_challenge"), code.challenge);
  assertEquals(url.searchParams.get("handoff_challenge"), handoff.challenge);
  assertEquals(url.href.includes(code.verifier), false);
  assertEquals(url.href.includes(handoff.verifier), false);
  assert(code.verifier !== handoff.verifier);
});

Deno.test("native OIDC polling preserves legacy and generic provider routes", () => {
  assertEquals(nativeOidcPollPath(cardea), "/api/auth/oidc/native/poll");
  assertEquals(nativeOidcEventsPath(cardea), "/api/auth/oidc/native/events");
  assertEquals(nativeOidcCancelPath(cardea), "/api/auth/oidc/native/cancel");
  assertEquals(
    nativeOidcPollPath({
      ...cardea,
      id: "google-workspace",
      start_url: "/api/auth/providers/google-workspace/start",
    }),
    "/api/auth/providers/google-workspace/native/poll",
  );
  assertEquals(
    nativeOidcEventsPath({
      ...cardea,
      id: "google-workspace",
      start_url: "/api/auth/providers/google-workspace/start",
    }),
    "/api/auth/providers/google-workspace/native/events",
  );
  assertEquals(
    nativeOidcCancelPath({
      ...cardea,
      id: "google-workspace",
      start_url: "/api/auth/providers/google-workspace/start",
    }),
    "/api/auth/providers/google-workspace/native/cancel",
  );
});

Deno.test("native OIDC WebSocket URL contains no PKCE proof", () => {
  const url = new URL(nativeOidcEventsUrl("https://cowboy.example", cardea));
  assertEquals(url.href, "wss://cowboy.example/api/auth/oidc/native/events");
  assertEquals(url.search, "");
  assertEquals(url.username, "");
  assertEquals(url.password, "");
});

Deno.test("closing the native browser cancels the local OIDC handoff", async () => {
  const root = globalThis as typeof globalThis & {
    __cowboyNativeShell?: boolean;
    __cowboyOpenAuthenticationBrowser?: (url: string) => boolean;
    __cowboyCloseAuthenticationBrowser?: () => void;
  };
  const previousFetch = globalThis.fetch;
  const previousLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const calls: Array<{ input: string; init?: RequestInit }> = [];
  let closes = 0;
  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL("https://cowboy.example/"),
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  root.__cowboyNativeShell = true;
  root.__cowboyOpenAuthenticationBrowser = () => {
    globalThis.dispatchEvent(
      new Event(NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT),
    );
    return true;
  };
  root.__cowboyCloseAuthenticationBrowser = () => closes += 1;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const path = typeof input === "string" ? input : input.toString();
    calls.push({ input: path, init });
    return Promise.resolve(new Response(null, { status: 204 }));
  }) as typeof fetch;
  try {
    await assertRejects(
      () => runNativeOidc(cardea),
      DOMException,
      "Cancelled",
    );
    assertEquals(calls.length, 1);
    assertEquals(calls[0]?.input, "/api/auth/oidc/native/cancel");
    assertEquals(calls[0]?.init?.method, "POST");
    const cancelled = JSON.parse(String(calls[0]?.init?.body)) as {
      handoff_token?: unknown;
      code_verifier?: unknown;
    };
    assertEquals(typeof cancelled.handoff_token, "string");
    assertEquals(typeof cancelled.code_verifier, "string");
    assert(cancelled.handoff_token !== cancelled.code_verifier);
    assertEquals(closes, 1);
  } finally {
    globalThis.fetch = previousFetch;
    delete root.__cowboyNativeShell;
    delete root.__cowboyOpenAuthenticationBrowser;
    delete root.__cowboyCloseAuthenticationBrowser;
    if (previousLocation) {
      Object.defineProperty(globalThis, "location", previousLocation);
    } else {
      delete (globalThis as { location?: Location }).location;
    }
    if (previousWindow) {
      Object.defineProperty(globalThis, "window", previousWindow);
    } else {
      delete (globalThis as { window?: Window }).window;
    }
  }
});

Deno.test("native OIDC waits on push and exchanges cookies exactly once", async () => {
  const root = globalThis as typeof globalThis & {
    __cowboyNativeShell?: boolean;
    __cowboyOpenAuthenticationBrowser?: (url: string) => boolean;
    __cowboyCloseAuthenticationBrowser?: () => void;
  };
  const previousFetch = globalThis.fetch;
  const previousLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const previousWebSocket = Object.getOwnPropertyDescriptor(
    globalThis,
    "WebSocket",
  );
  const fetches: Array<{ input: string; init?: RequestInit }> = [];
  const socketUrls: string[] = [];
  const socketMessages: string[] = [];
  const authenticationUrls: string[] = [];
  const completionOrder: string[] = [];
  let closes = 0;

  class FakeWebSocket extends EventTarget {
    constructor(url: string | URL) {
      super();
      socketUrls.push(String(url));
      queueMicrotask(() => this.dispatchEvent(new Event("open")));
    }

    send(data: string): void {
      socketMessages.push(data);
      queueMicrotask(() =>
        this.dispatchEvent(
          new MessageEvent("message", {
            data: JSON.stringify({ status: "ready" }),
          }),
        )
      );
    }

    close(): void {}
  }

  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL("https://cowboy.example/"),
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: FakeWebSocket,
  });
  root.__cowboyNativeShell = true;
  root.__cowboyOpenAuthenticationBrowser = (url) => {
    authenticationUrls.push(url);
    return true;
  };
  root.__cowboyCloseAuthenticationBrowser = () => {
    closes += 1;
    completionOrder.push("close");
    // Bridge v1 could report every dismissal as if the user closed the sheet.
    // Once the bound ready event arrives, that late signal must not cancel the
    // cookie exchange.
    globalThis.dispatchEvent(
      new Event(NATIVE_AUTHENTICATION_BROWSER_CLOSED_EVENT),
    );
  };
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const path = typeof input === "string" ? input : input.toString();
    completionOrder.push("poll");
    fetches.push({ input: path, init });
    return Promise.resolve(Response.json({ account: "draven", role: "owner" }));
  }) as typeof fetch;

  try {
    const me = await runNativeOidc(cardea);
    assertEquals(me, { account: "draven", role: "owner" });
    assertEquals(authenticationUrls.length, 1);
    const authenticationUrl = new URL(authenticationUrls[0]!);
    assertEquals(authenticationUrl.searchParams.has("code_verifier"), false);
    assertEquals(authenticationUrl.searchParams.has("handoff_token"), false);
    assertEquals(socketUrls, [
      "wss://cowboy.example/api/auth/oidc/native/events",
    ]);
    assertEquals(socketMessages.length, 1);
    const proofs = JSON.parse(socketMessages[0]!) as Record<string, string>;
    assertEquals(Object.keys(proofs).sort(), ["code_verifier", "handoff_token"]);
    assert(proofs.code_verifier !== proofs.handoff_token);
    assertEquals(socketUrls[0]?.includes(proofs.code_verifier), false);
    assertEquals(socketUrls[0]?.includes(proofs.handoff_token), false);
    assertEquals(fetches.length, 1);
    assertEquals(fetches[0]?.input, "/api/auth/oidc/native/poll");
    assertEquals(JSON.parse(String(fetches[0]?.init?.body)), proofs);
    assertEquals(closes, 2);
    assertEquals(completionOrder, ["close", "poll", "close"]);
  } finally {
    globalThis.fetch = previousFetch;
    delete root.__cowboyNativeShell;
    delete root.__cowboyOpenAuthenticationBrowser;
    delete root.__cowboyCloseAuthenticationBrowser;
    if (previousLocation) {
      Object.defineProperty(globalThis, "location", previousLocation);
    } else {
      delete (globalThis as { location?: Location }).location;
    }
    if (previousWindow) {
      Object.defineProperty(globalThis, "window", previousWindow);
    } else {
      delete (globalThis as { window?: Window }).window;
    }
    if (previousWebSocket) {
      Object.defineProperty(globalThis, "WebSocket", previousWebSocket);
    } else {
      delete (globalThis as { WebSocket?: typeof WebSocket }).WebSocket;
    }
  }
});

Deno.test("browser OIDC opens synchronously and preserves the pending SPA action", async () => {
  const previousFetch = globalThis.fetch;
  const previousLocation = Object.getOwnPropertyDescriptor(globalThis, "location");
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  const previousOpen = Object.getOwnPropertyDescriptor(globalThis, "open");
  const previousWebSocket = Object.getOwnPropertyDescriptor(
    globalThis,
    "WebSocket",
  );
  const navigations: string[] = [];
  const fetches: Array<{ input: string; init?: RequestInit }> = [];
  let popupOpener: unknown = globalThis;
  let popupOpened = 0;
  let popupClosed = 0;

  class FakeWebSocket extends EventTarget {
    constructor(_url: string | URL) {
      super();
      queueMicrotask(() => this.dispatchEvent(new Event("open")));
    }

    send(_data: string): void {
      queueMicrotask(() =>
        this.dispatchEvent(
          new MessageEvent("message", {
            data: JSON.stringify({ status: "ready" }),
          }),
        )
      );
    }

    close(): void {}
  }

  const popup = {
    get closed(): boolean {
      return false;
    },
    get opener(): unknown {
      return popupOpener;
    },
    set opener(value: unknown) {
      popupOpener = value;
    },
    close(): void {
      popupClosed += 1;
    },
    location: {
      replace(url: string): void {
        navigations.push(url);
      },
    },
  } as unknown as Window;

  Object.defineProperty(globalThis, "location", {
    configurable: true,
    value: new URL("https://cowboy.example/"),
  });
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  Object.defineProperty(globalThis, "open", {
    configurable: true,
    value: () => {
      popupOpened += 1;
      return popup;
    },
  });
  Object.defineProperty(globalThis, "WebSocket", {
    configurable: true,
    value: FakeWebSocket,
  });
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const path = typeof input === "string" ? input : input.toString();
    fetches.push({ input: path, init });
    return Promise.resolve(Response.json({ account: "draven", role: "owner" }));
  }) as typeof fetch;

  try {
    assertEquals(browserOidcFlowSupported(), true);
    const result = runBrowserOidc(cardea);
    assertEquals(popupOpened, 1);
    assertEquals(popupOpener, null);
    const me = await result;
    assertEquals(me, { account: "draven", role: "owner" });
    assertEquals(navigations.length, 1);
    const authorizationUrl = new URL(navigations[0]!);
    assertEquals(authorizationUrl.searchParams.get("client"), "browser-shell");
    assertEquals(authorizationUrl.searchParams.has("code_verifier"), false);
    assertEquals(authorizationUrl.searchParams.has("handoff_token"), false);
    assertEquals(fetches.length, 1);
    assertEquals(fetches[0]?.input, "/api/auth/oidc/native/poll");
    assertEquals(popupClosed, 1);
  } finally {
    globalThis.fetch = previousFetch;
    for (const [name, descriptor] of [
      ["location", previousLocation],
      ["window", previousWindow],
      ["open", previousOpen],
      ["WebSocket", previousWebSocket],
    ] as const) {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else delete (globalThis as Record<string, unknown>)[name];
    }
  }
});
