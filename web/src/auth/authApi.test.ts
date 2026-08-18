import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  AuthApiError,
  authApi,
  authStatusFromJson,
  fetchAuthStatus,
} from "./authApi.ts";

type FetchArgs = {
  input: string;
  init?: RequestInit;
};

function withFetch(
  handler: (args: FetchArgs) => Response | Promise<Response>,
): () => void {
  const previous = globalThis.fetch;
  globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string"
      ? input
      : input instanceof URL
      ? input.toString()
      : input.url;
    return Promise.resolve(handler({ input: url, init }));
  }) as typeof fetch;
  return () => {
    globalThis.fetch = previous;
  };
}

Deno.test("auth status fetch is same-origin and cache-free", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(
      JSON.stringify({
        registration: {
          enabled: false,
          mode: "disabled",
          accepts_registration: false,
        },
      }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  });
  try {
    const probe = await fetchAuthStatus();
    assertEquals(seen?.input, "/api/auth/status");
    assertEquals(seen?.init?.cache, "no-store");
    assertEquals(seen?.init?.credentials, "same-origin");
    assertEquals(probe.kind, "ok");
    if (probe.kind === "ok") {
      assertEquals(probe.body.me, undefined);
      assertEquals(probe.body.registration.accepts_registration, false);
    }
  } finally {
    restore();
  }
});

Deno.test("auth login and register POST JSON with credentials", async () => {
  const calls: FetchArgs[] = [];
  const restore = withFetch((args) => {
    calls.push(args);
    return new Response(
      JSON.stringify({ account: "draven", role: "operator" }),
      { status: 200 },
    );
  });
  try {
    assertEquals(await authApi.login("draven", "supersecret1"), {
      account: "draven",
      role: "operator",
    });
    assertEquals(await authApi.register("draven", "supersecret1", "invite"), {
      account: "draven",
      role: "operator",
    });
    assertEquals(calls[0]?.input, "/api/auth/login");
    assertEquals(calls[0]?.init?.method, "POST");
    assertEquals(calls[0]?.init?.credentials, "same-origin");
    assertEquals(calls[0]?.init?.body, JSON.stringify({
      account: "draven",
      password: "supersecret1",
    }));
    assertEquals(calls[1]?.input, "/api/auth/register");
    assertEquals(calls[1]?.init?.body, JSON.stringify({
      account: "draven",
      password: "supersecret1",
      token: "invite",
    }));
  } finally {
    restore();
  }
});

Deno.test("auth API surfaces HTTP error text", async () => {
  const restore = withFetch(() => new Response("invalid credentials", { status: 401 }));
  try {
    const error = await assertRejects(
      () => authApi.login("draven", "nope"),
      AuthApiError,
      "invalid credentials",
    );
    assertEquals(error.status, 401);
  } finally {
    restore();
  }
});

Deno.test("status 404 and 501 are unsupported, not login", async () => {
  for (const status of [404, 501] as const) {
    const restore = withFetch(() => new Response("not found", { status }));
    try {
      assertEquals(await fetchAuthStatus(), {
        kind: "unsupported",
        httpStatus: status,
      });
    } finally {
      restore();
    }
  }
});

Deno.test("status network and 5xx stay unavailable", async () => {
  const restoreNetwork = withFetch(() => {
    throw new TypeError("Failed to fetch");
  });
  try {
    assertEquals(await fetchAuthStatus(), { kind: "network" });
  } finally {
    restoreNetwork();
  }
  const restore5xx = withFetch(() => new Response("boom", { status: 502 }));
  try {
    assertEquals(await fetchAuthStatus(), {
      kind: "unavailable",
      httpStatus: 502,
    });
  } finally {
    restore5xx();
  }
});

Deno.test("logout is a same-origin POST", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  });
  try {
    assertEquals(await authApi.logout(), { ok: true });
    assertEquals(seen?.input, "/api/auth/logout");
    assertEquals(seen?.init?.method, "POST");
    assertEquals(seen?.init?.credentials, "same-origin");
    assertEquals(seen?.init?.cache, "no-store");
  } finally {
    restore();
  }
});

Deno.test("auth status JSON ignores a missing me and closed registration", () => {
  assertEquals(authStatusFromJson({}), {
    registration: {
      enabled: false,
      mode: "disabled",
      accepts_registration: false,
    },
  });
  assertEquals(
    authStatusFromJson({
      registration: { enabled: true, mode: "token", accepts_registration: true },
      me: { account: "draven", role: "operator" },
    }),
    {
      registration: { enabled: true, mode: "token", accepts_registration: true },
      me: { account: "draven", role: "operator" },
    },
  );
});
