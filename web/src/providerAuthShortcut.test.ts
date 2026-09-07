import { assertEquals } from "jsr:@std/assert";
import type { ProviderCatalogResponse } from "@cowboy/provider-ui";
import type { SessionMeta } from "./protocol.ts";
import { sessionProviderAuthShortcut } from "./providerAuthShortcut.ts";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

const session: SessionMeta = {
  id: "sess-auth",
  provider: "grok",
  provider_version: "3.1.8",
  provider_generation_digest: "digest",
  cwd: "/tmp",
  title: "Auth",
  status: "crashed",
};

function catalog(
  state: "signed_out" | "authenticating" | "ready" | "expired" | "error" | null,
  presentation: "account" | "api_key" = "account",
): ProviderCatalogResponse {
  return {
    providers: [{
      provider_id: "grok",
      provider_version: "3.1.8",
      package_digest: "package",
      artifact_digest: "digest",
      authentication_scope: "grok-auth-v1",
      release_state: "ready",
      publisher: "test",
      contract_fingerprint: "contract",
      supported_platforms: [],
      manifest: {
        display: { name: "Grok Build" },
        authentication: {
          required: true,
          presentation,
          methods: [],
        },
      },
    }],
    authentications: state === null ? [] : [{
      provider_id: "grok",
      authentication_scope: "grok-auth-v1",
      authentication_state: state,
    }],
    authentication_executors: [],
  } as unknown as ProviderCatalogResponse;
}

Deno.test("missing Provider auth replaces the downstream startup crash with a sign-in shortcut", () => {
  assertEquals(
    sessionProviderAuthShortcut(
      {
        sessionId: session.id,
        message: `worker ${session.id} entered Some(Crashed) before readiness`,
      },
      [session],
      catalog(null),
    ),
    {
      providerId: "grok",
      actionLabel: "Sign in",
      message: "Grok Build needs sign-in before this session can start.",
    },
  );
});

Deno.test("API-key Providers get typed recovery copy", () => {
  assertEquals(
    sessionProviderAuthShortcut(
      { sessionId: session.id, message: "Authentication required" },
      [session],
      catalog("expired", "api_key"),
    ),
    {
      providerId: "grok",
      actionLabel: "Add key",
      message: "Grok Build needs an API key before this session can start.",
    },
  );
});

Deno.test("ready or already-authenticating Providers do not start duplicate recovery", () => {
  for (const state of ["ready", "authenticating"] as const) {
    assertEquals(
      sessionProviderAuthShortcut(
        { sessionId: session.id, message: "unrelated failure" },
        [session],
        catalog(state),
      ),
      null,
    );
  }
});

Deno.test("unrecognized failures keep their original detail beside the shortcut", () => {
  assertEquals(
    sessionProviderAuthShortcut(
      { sessionId: session.id, message: "workspace is unavailable" },
      [session],
      catalog("signed_out"),
    )?.message,
    "workspace is unavailable",
  );
});

Deno.test("the error snackbar opens the focused Provider sign-in flow", () => {
  assertEquals(appSource.includes("sessionProviderAuthShortcut("), true);
  assertEquals(
    appSource.includes('openSettings("providers", undefined, {'),
    true,
  );
  assertEquals(appSource.includes("autoBeginAuthentication: true"), true);
  assertEquals(appSource.includes("providerAuthShortcut.actionLabel"), true);
});
