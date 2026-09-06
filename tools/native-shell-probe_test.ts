const source = await Deno.readTextFile(
  new URL("./native-shell-probe.js", import.meta.url),
);
const probe = new Function(
  "window",
  "location",
  "navigator",
  "document",
  "fetch",
  "remote",
  source + "\nreturn probeCowboyNativeShell(remote);",
);

function assert(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function rejects(run: () => Promise<unknown>, message: string) {
  try {
    await run();
  } catch (error) {
    assert(String(error).includes(message), String(error));
    return;
  }
  throw new Error("expected rejection: " + message);
}

function fixture() {
  const invocations: string[] = [];
  const requests: [string, RequestInit][] = [];
  const context = {
    origin: "https://cowboy.stormbird.xyz",
    ready: "complete",
    heading: "Sign in",
    headingHeight: 30,
    auth: {
      registration: {
        enabled: true,
        mode: "disabled",
        accepts_registration: false,
      },
      me: null,
    } as Record<string, unknown>,
    contentType: "application/json",
    httpStatus: 200,
    haptics: true,
    allowSettings: false,
    missingOpener: false,
  };
  const browser = {
    __cowboyNativeShell: true,
    __cowboySelectionHaptic() {
      throw new Error("must not use the legacy haptic shim");
    },
    __cowboyReadClipboard() {
      throw new Error("must not read the clipboard");
    },
    __cowboyAuthenticationBrowserBridgeVersion: 2,
    __COWBOY_NATIVE_PLUGIN_HOST: Object.freeze({
      version: "1.0.0",
      capabilities: Object.freeze(["webauthn"]),
      invoke(name: string, args: Record<string, string>) {
        if (name === "unknown") {
          return Promise.reject(new Error("unknown capability"));
        }
        assert(
          name === "webauthn" && args.action === "capabilities",
          "no credential ceremony",
        );
        return Promise.resolve({ ok: true, available: false });
      },
    }),
    __TAURI__: {
      core: {
        invoke(name: string, args?: { url: string }) {
          invocations.push(name);
          if (name === "plugin:haptics|selection_feedback") {
            if (!context.haptics) {
              return Promise.reject(new Error("haptics not allowed"));
            }
            return Promise.resolve(null);
          }
          assert(
            name === "plugin:opener|open_url",
            "unexpected native command",
          );
          if (context.missingOpener) return Promise.reject("Command not found");
          if (args?.url === "app-settings:" && context.allowSettings) {
            return Promise.resolve(null);
          }
          return Promise.reject("Not allowed to open url " + args?.url);
        },
      },
    },
  };
  const run = (remote: boolean) =>
    probe(
      browser,
      { origin: context.origin },
      { userAgent: "iPhone AppleWebKit" },
      {
        readyState: context.ready,
        querySelector(selector: string) {
          assert(
            selector === "form h1",
            "probe must only inspect the login heading",
          );
          return {
            textContent: context.heading,
            getBoundingClientRect: () => ({ height: context.headingHeight }),
          };
        },
      },
      (url: string, options: RequestInit) => {
        requests.push([url, options]);
        assert(remote, "basic shell acceptance must not make an auth request");
        assert(
          url === "/api/auth/status" && options.method === "GET",
          "read-only status only",
        );
        assert(
          options.credentials === "omit" && options.redirect === "error",
          "no credentials or redirects",
        );
        assert(
          options.cache === "no-store" && !options.body,
          "no cached status or login body",
        );
        return Promise.resolve({
          status: context.httpStatus,
          headers: new Headers({ "content-type": context.contentType }),
          json: () => Promise.resolve(context.auth),
        });
      },
      remote,
    );
  return { context, browser, run, invocations, requests };
}

Deno.test("basic shell probe does not claim a remote page or request authentication", async () => {
  const test = fixture();
  test.context.origin = "tauri://localhost";
  const report = await test.run(false);
  assert(
    report.tests.length === 7 && report.phase === "shell",
    "basic probe contract",
  );
  assert(test.requests.length === 0, "unexpected network call");
});

Deno.test("remote probe exercises positive IPC and checks only public logged-out status", async () => {
  const test = fixture();
  const report = await test.run(true);
  assert(
    report.tests.length === 15 && report.phase === "remote-logged-out",
    "remote probe contract",
  );
  assert(new Set(report.tests).size === 15, "duplicate checks");
  assert(test.requests.length === 1, "expected one read-only request");
  assert(
    test.invocations.includes("plugin:haptics|selection_feedback"),
    "must prove positive remote IPC",
  );
});

Deno.test("local loader, foreign origin and wrong port cannot pass remote acceptance", async () => {
  for (
    const origin of [
      "tauri://localhost",
      "https://foreign.invalid",
      "https://cowboy.stormbird.xyz:444",
    ]
  ) {
    const test = fixture();
    test.context.origin = origin;
    await rejects(() => test.run(true), "remote Cowboy origin");
    assert(
      test.requests.length === 0 && test.invocations.length === 0,
      "wrong-origin effects",
    );
  }
});

Deno.test("remote navigation alone cannot pass without rendered login UI", async () => {
  for (
    const change of [{ ready: "loading" }, { heading: "Cowboy" }, {
      headingHeight: 0,
    }]
  ) {
    const test = fixture();
    Object.assign(test.context, change);
    await rejects(
      () => test.run(true),
      change.ready ? "remote document ready" : "remote sign-in UI rendered",
    );
    assert(test.requests.length === 0, "must wait for the actual page");
  }
});

Deno.test("remote probe rejects an authenticated or auth-disabled server response", async () => {
  const test = fixture();
  test.context.auth.me = {
    account: "fixture",
    role: "owner",
    auth_enabled: false,
  };
  await rejects(() => test.run(true), "remote remains logged out");
});

Deno.test("HTML, unsuccessful or incompatible status responses are not login acceptance", async () => {
  for (
    const change of [{ contentType: "text/html" }, { httpStatus: 503 }, {
      auth: {},
    }]
  ) {
    const test = fixture();
    Object.assign(test.context, change);
    await rejects(
      () => test.run(true),
      change.auth ? "auth status contract" : "read-only auth status is JSON",
    );
  }
});

Deno.test("a globally injected but unauthorized remote IPC bridge is rejected", async () => {
  const test = fixture();
  test.context.haptics = false;
  await rejects(() => test.run(true), "haptics not allowed");
});

Deno.test("remote settings access cannot inherit the local loader permission", async () => {
  const test = fixture();
  test.context.allowSettings = true;
  await rejects(() => test.run(true), "remote cannot open app settings");
});

Deno.test("missing opener plugin is not mistaken for a valid scope rejection", async () => {
  const test = fixture();
  test.context.missingOpener = true;
  await rejects(() => test.run(true), "opener rejects local files");
});

Deno.test("ordinary remote WebKit cannot masquerade as the native shell", async () => {
  const test = fixture();
  test.browser.__cowboyNativeShell = false;
  await rejects(() => test.run(true), "native keyboard shell");
});
