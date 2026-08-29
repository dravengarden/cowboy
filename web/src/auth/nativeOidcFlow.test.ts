import { assert, assertEquals } from "jsr:@std/assert";
import {
  nativeOidcPollPath,
  type ProductOidcProvider,
} from "./authApi.ts";
import { nativeOidcStartUrl } from "./nativeOidcFlow.ts";
import { newPkceBinding } from "./pkce.ts";

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
  assertEquals(
    nativeOidcPollPath({
      ...cardea,
      id: "google-workspace",
      start_url: "/api/auth/providers/google-workspace/start",
    }),
    "/api/auth/providers/google-workspace/native/poll",
  );
});
