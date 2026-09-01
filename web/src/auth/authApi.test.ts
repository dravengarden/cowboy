import { assertEquals, assertRejects } from "jsr:@std/assert";
import {
  authApi,
  AuthApiError,
  authStatusFromJson,
  externalPasskeyApi,
  fetchAuthStatus,
  isHtmlContentType,
} from "./authApi.ts";
import { classifyAuthStatus, isLoginDecision } from "./authStatus.ts";

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
    if (args.input === "/api/auth/setup") {
      return new Response(
        JSON.stringify({
          registration: {
            enabled: false,
            mode: "disabled",
            accepts_registration: false,
          },
          setup_required: true,
          setup_pending: true,
        }),
        { status: 200 },
      );
    }
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
    assertEquals(await authApi.setup("cow_setup_test"), {
      registration: {
        enabled: false,
        mode: "disabled",
        accepts_registration: false,
      },
      setup_required: true,
      setup_pending: true,
    });
    assertEquals(await authApi.register("draven", "supersecret1"), {
      account: "draven",
      role: "operator",
    });
    assertEquals(calls[0]?.input, "/api/auth/login");
    assertEquals(calls[0]?.init?.method, "POST");
    assertEquals(calls[0]?.init?.credentials, "same-origin");
    assertEquals(calls[0]?.init?.cache, "no-store");
    const loginHeaders = calls[0]?.init?.headers as
      | Record<string, string>
      | undefined;
    assertEquals(loginHeaders?.["content-type"], "application/json");
    assertEquals(loginHeaders?.accept, "application/json");
    assertEquals(
      calls[0]?.init?.body,
      JSON.stringify({
        account: "draven",
        password: "supersecret1",
      }),
    );
    assertEquals(calls[1]?.input, "/api/auth/setup");
    assertEquals(
      calls[1]?.init?.body,
      JSON.stringify({
        token: "cow_setup_test",
      }),
    );
    assertEquals(calls[2]?.input, "/api/auth/register");
    assertEquals(
      calls[2]?.init?.body,
      JSON.stringify({
        account: "draven",
        password: "supersecret1",
      }),
    );
  } finally {
    restore();
  }
});

Deno.test("auth API surfaces HTTP error text", async () => {
  const restore = withFetch(() =>
    new Response("invalid credentials", { status: 401 })
  );
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

Deno.test("external Passkey handoff keeps the verifier in the signed-in client", async () => {
  const calls: FetchArgs[] = [];
  const restore = withFetch((args) => {
    calls.push(args);
    if (args.input.endsWith("/start")) {
      return new Response(
        JSON.stringify({
          transaction_id: "a".repeat(64),
          expires_in_seconds: 120,
        }),
        { status: 200 },
      );
    }
    return new Response(
      JSON.stringify({
        status: "ready",
        action: "register",
        publicKey: {},
      }),
      { status: 200 },
    );
  });
  try {
    await authApi.startExternalPasskey("register", "c".repeat(43), "iPhone");
    await externalPasskeyApi.options("a".repeat(64));
    const started = calls[0];
    const browser = calls[1];
    assertEquals(started.init?.credentials, "same-origin");
    assertEquals(browser.init?.credentials, "omit");
    assertEquals(browser.init?.referrerPolicy, "no-referrer");
    const body = JSON.parse(String(started.init?.body));
    assertEquals(body.code_challenge, "c".repeat(43));
    assertEquals("code_verifier" in body, false);
  } finally {
    restore();
  }
});

Deno.test("external Passkey finalize forwards the transaction deadline signal", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(JSON.stringify({ status: "pending" }), { status: 200 });
  });
  const flow = new AbortController();
  try {
    await authApi.finalizeExternalPasskey(
      "a".repeat(64),
      "v".repeat(64),
      flow.signal,
    );
    assertEquals(seen?.init?.signal, flow.signal);
  } finally {
    restore();
  }
});

Deno.test("native OIDC poll keeps both raw bindings in a same-origin body", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(
      JSON.stringify({ account: "draven", role: "owner" }),
      { status: 200 },
    );
  });
  try {
    assertEquals(
      await authApi.pollNativeOidc(
        {
          id: "cardea",
          display_name: "Cardea",
          button_label: "Continue with Cardea",
          start_url: "/api/auth/oidc/start",
        },
        "h".repeat(43),
        "v".repeat(43),
      ),
      { account: "draven", role: "owner" },
    );
    assertEquals(seen?.input, "/api/auth/oidc/native/poll");
    assertEquals(seen?.init?.method, "POST");
    assertEquals(seen?.init?.credentials, "same-origin");
    assertEquals(seen?.init?.cache, "no-store");
    assertEquals(
      seen?.init?.body,
      JSON.stringify({
        handoff_token: "h".repeat(43),
        code_verifier: "v".repeat(43),
      }),
    );
    assertEquals(String(seen?.input).includes("h".repeat(43)), false);
    assertEquals(String(seen?.input).includes("v".repeat(43)), false);
  } finally {
    restore();
  }
});

Deno.test("native OIDC cancellation is same-origin and keeps proofs out of the URL", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(null, { status: 204 });
  });
  try {
    await authApi.cancelNativeOidc(
      {
        id: "google-workspace",
        display_name: "Google Workspace",
        button_label: "Continue with Google",
        start_url: "/api/auth/providers/google-workspace/start",
      },
      "h".repeat(43),
      "v".repeat(43),
    );
    assertEquals(
      seen?.input,
      "/api/auth/providers/google-workspace/native/cancel",
    );
    assertEquals(seen?.init?.method, "POST");
    assertEquals(seen?.init?.credentials, "same-origin");
    assertEquals(
      seen?.init?.body,
      JSON.stringify({
        handoff_token: "h".repeat(43),
        code_verifier: "v".repeat(43),
      }),
    );
    assertEquals(String(seen?.input).includes("h".repeat(43)), false);
    assertEquals(String(seen?.input).includes("v".repeat(43)), false);
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

Deno.test("legacy token CRUD remains same-origin and never sends the hash", async () => {
  const calls: FetchArgs[] = [];
  const restore = withFetch((args) => {
    calls.push(args);
    if (args.init?.method === "POST") {
      return new Response(
        JSON.stringify({
          id: "tok1",
          name: "zed",
          token: "cow_secret",
          token_prefix: "cow_secr",
          created_at_ms: 1,
        }),
        { status: 200 },
      );
    }
    if (args.init?.method === "DELETE") {
      return new Response(JSON.stringify({ ok: true }), { status: 200 });
    }
    return new Response(
      JSON.stringify({
        tokens: [{
          id: "tok1",
          name: "zed",
          token_prefix: "cow_secr",
          created_at_ms: 1,
        }],
      }),
      { status: 200 },
    );
  });
  try {
    assertEquals(await authApi.listTokens(), {
      tokens: [{
        id: "tok1",
        name: "zed",
        token_prefix: "cow_secr",
        created_at_ms: 1,
      }],
    });
    assertEquals((await authApi.createToken("zed")).token, "cow_secret");
    assertEquals(await authApi.deleteToken("tok1"), { ok: true });
    assertEquals(calls[0]?.input, "/api/auth/tokens");
    assertEquals(calls[0]?.init?.credentials, "same-origin");
    assertEquals(calls[1]?.input, "/api/auth/tokens");
    assertEquals(calls[1]?.init?.method, "POST");
    assertEquals(calls[1]?.init?.body, JSON.stringify({ name: "zed" }));
    assertEquals(calls[2]?.input, "/api/auth/tokens/tok1");
    assertEquals(calls[2]?.init?.method, "DELETE");
  } finally {
    restore();
  }
});

Deno.test("device authorization keeps the capability off authenticated requests", async () => {
  const calls: FetchArgs[] = [];
  const request = {
    request_id: "request_abcdefghijklmnopqrstuvwxyz",
    approval_token: "approval_abcdefghijklmnopqrstuvwxyz",
  };
  const restore = withFetch((args) => {
    calls.push(args);
    if (args.input.endsWith("/inspect")) {
      return new Response(
        JSON.stringify({
          request_id: request.request_id,
          name: "Zed on Hawk",
          fingerprint: "SHA256:test",
          expires_at_ms: 1_900_000_000_000,
          status: "pending",
        }),
        { status: 200 },
      );
    }
    if (args.input === "/api/auth/devices") {
      return new Response(
        JSON.stringify({
          devices: [{
            id: "device-1",
            name: "Zed on Hawk",
            created_at_ms: 1_900_000_000_000,
          }],
        }),
        { status: 200 },
      );
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  });
  try {
    assertEquals(
      (await authApi.inspectDeviceAuthorization(request)).status,
      "pending",
    );
    assertEquals(await authApi.approveDeviceAuthorization(request), {
      ok: true,
    });
    assertEquals(await authApi.denyDeviceAuthorization(request), { ok: true });
    assertEquals((await authApi.listDevices()).devices[0]?.id, "device-1");
    assertEquals(await authApi.deleteDevice("device/1"), { ok: true });

    assertEquals(
      calls[0]?.input,
      "/api/auth/device/authorizations/inspect",
    );
    assertEquals(calls[0]?.init?.credentials, "omit");
    assertEquals(calls[0]?.init?.referrerPolicy, "no-referrer");
    assertEquals(calls[0]?.init?.body, JSON.stringify(request));
    assertEquals(calls[1]?.init?.credentials, "same-origin");
    assertEquals(calls[2]?.init?.credentials, "same-origin");
    assertEquals(calls[3]?.input, "/api/auth/devices");
    assertEquals(calls[4]?.input, "/api/auth/devices/device%2F1");
    assertEquals(calls[4]?.init?.method, "DELETE");
  } finally {
    restore();
  }
});

Deno.test("logout is a same-origin POST", async () => {
  let seen: FetchArgs | undefined;
  const restore = withFetch((args) => {
    seen = args;
    return new Response(
      JSON.stringify({ ok: true, scope: "current", revoked_sessions: 1 }),
      { status: 200 },
    );
  });
  try {
    assertEquals(await authApi.logout(), {
      ok: true,
      scope: "current",
      revoked_sessions: 1,
    });
    assertEquals(seen?.input, "/api/auth/logout");
    assertEquals(seen?.init?.method, "POST");
    assertEquals(seen?.init?.credentials, "same-origin");
    assertEquals(seen?.init?.cache, "no-store");
  } finally {
    restore();
  }
});

Deno.test("session capacity inventory and reclaim requests preserve fencing", async () => {
  const calls: FetchArgs[] = [];
  const restore = withFetch((args) => {
    calls.push(args);
    if (args.input === "/api/auth/sessions") {
      return new Response(
        JSON.stringify({
          sessions: [{
            id: "session-1",
            current: true,
            client_kind: "browser",
            principal_class: "human",
            created_at_ms: 1,
            expires_at_ms: 10,
            last_seen_at_ms: 2,
          }],
          active_clients: [{
            client_id: "client/a",
            user_id: "user-1",
            principal_class: "human",
            session_id: "session-1",
            client_kind: "browser",
            fencing_token: 17,
            acquired_at_ms: 1,
            heartbeat_at_ms: 2,
            expires_at_ms: 10,
          }],
          authorized_clients: 2,
          limit: 10,
          active_limit: 4,
          enforcement: "enforce",
        }),
        { status: 200 },
      );
    }
    return new Response(JSON.stringify({ ok: true }), { status: 200 });
  });
  try {
    const inventory = await authApi.listSessions();
    assertEquals(inventory.active_clients[0]?.fencing_token, 17);
    assertEquals(inventory.authorized_clients, 2);
    assertEquals(await authApi.deleteSession("session/2"), { ok: true });
    assertEquals(await authApi.releaseActiveClient("client/a", 17), {
      ok: true,
    });
    assertEquals(calls[0]?.input, "/api/auth/sessions");
    assertEquals(calls[0]?.init?.credentials, "same-origin");
    assertEquals(calls[1]?.input, "/api/auth/sessions/session%2F2");
    assertEquals(calls[1]?.init?.method, "DELETE");
    assertEquals(
      calls[2]?.input,
      "/api/auth/active-clients/client%2Fa?fencing_token=17",
    );
    assertEquals(calls[2]?.init?.method, "DELETE");
  } finally {
    restore();
  }
});

Deno.test("auth status JSON requires the public registration shape", () => {
  assertEquals(authStatusFromJson({}), undefined);
  assertEquals(authStatusFromJson("<!doctype html>"), undefined);
  assertEquals(
    authStatusFromJson({
      registration: {
        enabled: true,
        mode: "token",
        accepts_registration: true,
      },
      me: {
        account: "draven",
        role: "operator",
        auth_enabled: false,
        primary_auth_method: "cardea",
      },
    }),
    {
      registration: {
        enabled: true,
        mode: "token",
        accepts_registration: true,
      },
      setup_required: false,
      setup_pending: false,
      password_enabled: true,
      login_method_order: ["password"],
      providers: [],
      me: {
        account: "draven",
        role: "operator",
        auth_enabled: false,
        primary_auth_method: "cardea",
      },
    },
  );
});

Deno.test("auth status accepts pinned provider routes and server Passkey policy", () => {
  assertEquals(
    authStatusFromJson({
      registration: {
        enabled: false,
        mode: "disabled",
        accepts_registration: false,
      },
      password_enabled: false,
      login_method_order: ["google", "cardea"],
      passkeys: {
        enabled: true,
        prompt_after_login: true,
        session_refresh_enabled: false,
      },
      session: {
        activity_sliding_enabled: true,
        idle_timeout_ms: 86_400_000,
        passkey_max_age_ms: 604_800_000,
        passkey_warning_ms: 1_800_000,
        primary_max_age_ms: 2_592_000_000,
        primary_warning_ms: 86_400_000,
      },
      capacity: {
        enforcement: "enforce",
        authorized_clients_per_user: 12,
        signed_in_sessions_per_user: 10,
        active_clients_per_user: 4,
        active_clients_service: 32,
        websocket_channels_per_client: 8,
        active_lease_ms: 120_000,
        heartbeat_ms: 30_000,
        reservation_ms: 30_000,
        session_overflow: "revoke_oldest_inactive",
        active_overflow: "wait_or_reclaim_own",
        single_session_mode: "off",
      },
      logout: {
        provider_logout: "offer",
        backchannel_logout: true,
      },
      automation: {
        enabled: false,
        active_clients: 32,
        credential_max_age_ms: 600_000,
      },
      providers: [
        {
          id: "cardea",
          display_name: "Cardea",
          button_label: "Continue with Cardea",
          start_url: "/api/auth/oidc/start",
        },
        {
          id: "google",
          display_name: "Google",
          button_label: "Continue with Google",
          start_url: "/api/auth/providers/google/start",
        },
        {
          id: "forged",
          display_name: "Forged",
          button_label: "Continue",
          start_url: "/api/auth/providers/google/start",
        },
      ],
    }),
    {
      registration: {
        enabled: false,
        mode: "disabled",
        accepts_registration: false,
      },
      setup_required: false,
      setup_pending: false,
      password_enabled: false,
      login_method_order: ["google", "cardea"],
      passkeys: {
        enabled: true,
        prompt_after_login: true,
        session_refresh_enabled: false,
      },
      session: {
        activity_sliding_enabled: true,
        idle_timeout_ms: 86_400_000,
        passkey_max_age_ms: 604_800_000,
        passkey_warning_ms: 1_800_000,
        primary_max_age_ms: 2_592_000_000,
        primary_warning_ms: 86_400_000,
      },
      capacity: {
        enforcement: "enforce",
        authorized_clients_per_user: 12,
        signed_in_sessions_per_user: 10,
        active_clients_per_user: 4,
        active_clients_service: 32,
        websocket_channels_per_client: 8,
        active_lease_ms: 120_000,
        heartbeat_ms: 30_000,
        reservation_ms: 30_000,
        session_overflow: "revoke_oldest_inactive",
        active_overflow: "wait_or_reclaim_own",
        single_session_mode: "off",
      },
      logout: {
        provider_logout: "offer",
        backchannel_logout: true,
      },
      automation: {
        enabled: false,
        active_clients: 32,
        credential_max_age_ms: 600_000,
      },
      providers: [
        {
          id: "cardea",
          display_name: "Cardea",
          button_label: "Continue with Cardea",
          start_url: "/api/auth/oidc/start",
        },
        {
          id: "google",
          display_name: "Google",
          button_label: "Continue with Google",
          start_url: "/api/auth/providers/google/start",
        },
      ],
    },
  );
});

Deno.test("auth status defaults Cardea first and rejects incomplete method orders", () => {
  const base = {
    registration: {
      enabled: false,
      mode: "disabled",
      accepts_registration: false,
    },
    password_enabled: true,
    providers: [
      {
        id: "cardea",
        display_name: "Cardea",
        button_label: "Continue with Cardea",
        start_url: "/api/auth/oidc/start",
      },
      {
        id: "google",
        display_name: "Google",
        button_label: "Continue with Google",
        start_url: "/api/auth/providers/google/start",
      },
    ],
  } as const;
  assertEquals(
    authStatusFromJson(base)?.login_method_order,
    ["cardea", "password", "google"],
  );
  assertEquals(
    authStatusFromJson({
      ...base,
      login_method_order: ["password", "google", "cardea"],
    })?.login_method_order,
    ["password", "google", "cardea"],
  );
  assertEquals(
    authStatusFromJson({
      ...base,
      login_method_order: ["cardea", "password"],
    })?.login_method_order,
    ["cardea", "password", "google"],
  );
});

Deno.test("200 HTML or shapeless JSON is activating, not login", async () => {
  assertEquals(isHtmlContentType("text/html; charset=utf-8"), true);
  const restoreHtml = withFetch(() =>
    new Response("<!doctype html><title>Cowboy</title>", {
      status: 200,
      headers: { "content-type": "text/html; charset=utf-8" },
    })
  );
  try {
    const probe = await fetchAuthStatus();
    assertEquals(probe, { kind: "unsupported", httpStatus: 200 });
    const decision = classifyAuthStatus(probe);
    assertEquals(decision.view, "activating");
    assertEquals(isLoginDecision(decision), false);
  } finally {
    restoreHtml();
  }
  const restoreEmpty = withFetch(() =>
    new Response("{}", {
      status: 200,
      headers: { "content-type": "application/json" },
    })
  );
  try {
    assertEquals(await fetchAuthStatus(), {
      kind: "unsupported",
      httpStatus: 200,
    });
  } finally {
    restoreEmpty();
  }
  const restoreText = withFetch(() =>
    new Response("not json", {
      status: 200,
      headers: { "content-type": "text/plain" },
    })
  );
  try {
    assertEquals(await fetchAuthStatus(), {
      kind: "unsupported",
      httpStatus: 200,
    });
  } finally {
    restoreText();
  }
});
