import { assertEquals } from "jsr:@std/assert";
import {
  classifyAuthStatus,
  deleteProductHistoryCache,
  historyCacheName,
  isLoginDecision,
  nextAuthStatusBackoffMs,
  shouldMountProductApp,
  shouldOpenWebSocket,
  showRegistration,
  showRegistrationToken,
} from "./authStatus.ts";

const closed = {
  enabled: false,
  mode: "disabled" as const,
  accepts_registration: false,
};

const tokenOpen = {
  enabled: true,
  mode: "token" as const,
  accepts_registration: true,
};

Deno.test("200 with me mounts the product apps and may open /ws", () => {
  const decision = classifyAuthStatus({
    kind: "ok",
    httpStatus: 200,
    body: {
      registration: closed,
      me: { account: "draven", role: "operator" },
    },
  });
  assertEquals(decision.view, "ready");
  assertEquals(shouldMountProductApp(decision), true);
  assertEquals(shouldOpenWebSocket(decision), true);
  assertEquals(isLoginDecision(decision), false);
});

Deno.test("200 without me is login and must not open /ws", () => {
  const decision = classifyAuthStatus({
    kind: "ok",
    httpStatus: 200,
    body: { registration: closed },
  });
  assertEquals(decision.view, "login");
  assertEquals(shouldMountProductApp(decision), false);
  assertEquals(shouldOpenWebSocket(decision), false);
  assertEquals(isLoginDecision(decision), true);
});

Deno.test("404 and 501 are activating, never login-forever", () => {
  for (const httpStatus of [404, 501] as const) {
    const decision = classifyAuthStatus({
      kind: "unsupported",
      httpStatus,
    });
    assertEquals(decision.view, "activating");
    assertEquals(isLoginDecision(decision), false);
    assertEquals(shouldMountProductApp(decision), false);
    assertEquals(shouldOpenWebSocket(decision), false);
  }
});

Deno.test("network and 5xx retry without clearing login state", () => {
  assertEquals(classifyAuthStatus({ kind: "network" }).view, "retry");
  assertEquals(
    classifyAuthStatus({ kind: "unavailable", httpStatus: 503 }).view,
    "retry",
  );
  assertEquals(
    shouldOpenWebSocket(classifyAuthStatus({ kind: "network" })),
    false,
  );
});

Deno.test("register chrome follows accepts_registration and token mode", () => {
  assertEquals(showRegistration(closed), false);
  assertEquals(showRegistrationToken(closed), false);
  assertEquals(showRegistration(tokenOpen), true);
  assertEquals(showRegistrationToken(tokenOpen), true);
  assertEquals(
    showRegistrationToken({
      enabled: true,
      mode: "open",
      accepts_registration: true,
    }),
    false,
  );
});

Deno.test("status retry backoff matches the connection banner", () => {
  assertEquals(nextAuthStatusBackoffMs(1), 1000);
  assertEquals(nextAuthStatusBackoffMs(2), 2000);
  assertEquals(nextAuthStatusBackoffMs(3), 4000);
  assertEquals(nextAuthStatusBackoffMs(4), 8000);
  assertEquals(nextAuthStatusBackoffMs(5), 15000);
  assertEquals(nextAuthStatusBackoffMs(8), 15000);
});

Deno.test("logout deletes only HISTORY_CACHE generations", async () => {
  assertEquals(historyCacheName("cowboy-v1405"), "cowboy-v1405-history");
  const deleted: string[] = [];
  await deleteProductHistoryCache({
    keys: () =>
      Promise.resolve([
        "cowboy-v1405-history",
        "cowboy-v1404-history",
        "cowboy-v1405-assets",
        "other-app-history",
      ]),
    delete: (key) => {
      deleted.push(key);
      return Promise.resolve(true);
    },
  });
  assertEquals(deleted, ["cowboy-v1405-history", "cowboy-v1404-history"]);
});
