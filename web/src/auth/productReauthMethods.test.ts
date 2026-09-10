import { assertEquals } from "jsr:@std/assert";
import type { AuthHostPlugin, ProductOidcProvider } from "./authApi.ts";
import {
  loginMethodLabel,
  productAccountVerificationMethods,
  resolvePrimaryReauthMethods,
} from "./productReauthMethods.ts";

const providers: ProductOidcProvider[] = [{
  id: "cardea",
  display_name: "Cardea",
  button_label: "Continue with Cardea",
  start_url: "/api/auth/oidc/start",
}];
const hostPlugins: AuthHostPlugin[] = [
  { id: "password", slots: ["login.method"], label: "Password" },
  { id: "cardea", slots: ["login.method"], label: "Cardea SSO" },
];
const accountMethods = productAccountVerificationMethods(
  ["cardea", "password"],
  true,
  providers,
  hostPlugins,
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
      label: "Cardea SSO",
      authMethod: "cardea",
    }],
    legacySession: false,
  });
});

Deno.test("only external login method labels may use a host Plugin", () => {
  assertEquals(
    loginMethodLabel("password", hostPlugins, providers),
    "Password",
  );
  assertEquals(
    loginMethodLabel("cardea", hostPlugins, providers),
    "Cardea SSO",
  );
  assertEquals(loginMethodLabel("password", [], providers), "Password");
  assertEquals(
    loginMethodLabel("password", [{
      id: "password",
      slots: ["login.method"],
      label: "Misleading prompt",
    }], providers),
    "Password",
  );
  assertEquals(loginMethodLabel("cardea", [], providers), "Cardea");
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
