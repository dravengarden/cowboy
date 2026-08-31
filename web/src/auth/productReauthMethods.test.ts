import { assertEquals } from "jsr:@std/assert";
import type { ProductOidcProvider } from "./authApi.ts";
import {
  productAccountVerificationMethods,
  resolvePrimaryReauthMethods,
} from "./productReauthMethods.ts";

const providers: ProductOidcProvider[] = [{
  id: "cardea",
  display_name: "Cardea",
  button_label: "Continue with Cardea",
  start_url: "/api/auth/oidc/start",
}];
const accountMethods = productAccountVerificationMethods(
  ["cardea", "password"],
  true,
  providers,
);

Deno.test("primary reauthentication keeps the session's password method", () => {
  assertEquals(resolvePrimaryReauthMethods("password", accountMethods), {
    methods: [{
      id: "password",
      label: "Password",
      authMethod: "password",
    }],
    legacySession: false,
  });
});

Deno.test("primary reauthentication keeps the session's provider method", () => {
  assertEquals(resolvePrimaryReauthMethods("cardea", accountMethods), {
    methods: [{
      id: "provider:cardea",
      label: "Cardea",
      authMethod: "cardea",
    }],
    legacySession: false,
  });
});

Deno.test("legacy sessions choose once while disabled methods cannot switch", () => {
  assertEquals(resolvePrimaryReauthMethods(null, accountMethods), {
    methods: accountMethods,
    legacySession: true,
  });
  assertEquals(resolvePrimaryReauthMethods("google", accountMethods), {
    methods: [],
    legacySession: false,
    unavailableMethod: "google",
  });
});
