import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  corePasskeysEnabled,
  corePasswordMode,
  passwordLoginFields,
} from "./coreSecurity.ts";

test("local bootstrap is independent from Catalog and the ordinary login method", () => {
  for (const passwordEnabled of [false, true]) {
    for (const selectedMethod of ["password", "cardea", "missing", ""]) {
      assertEquals(
        corePasswordMode({
          setupRequired: true,
          setupPending: false,
          passwordEnabled,
          selectedMethod,
        }),
        "setup",
      );
      assertEquals(
        corePasswordMode({
          setupRequired: true,
          setupPending: true,
          passwordEnabled,
          selectedMethod,
        }),
        "register",
      );
    }
  }
});

test("a disabled, foreign or stale local login selection cannot submit a password", () => {
  for (const selectedMethod of ["password", "cardea", "missing", ""]) {
    for (const setupPending of [false, true]) {
      assertEquals(
        corePasswordMode({
          setupRequired: false,
          setupPending,
          passwordEnabled: false,
          selectedMethod,
        }),
        null,
      );
      assertEquals(
        corePasswordMode({
          setupRequired: false,
          setupPending,
          passwordEnabled: true,
          selectedMethod,
        }),
        selectedMethod === "password" ? "login" : null,
      );
    }
  }
});

test("local security presentation uses core copy and explicit Service policy", () => {
  const fields = passwordLoginFields();
  assertEquals(fields, {
    account: "Account",
    secret: "Password",
    confirm: "Confirm password",
    setup: "Setup code",
  });
  fields.secret = "a caller cannot change the next core form";
  assertEquals(passwordLoginFields().secret, "Password");
  assertEquals(corePasskeysEnabled(undefined), false);
  assertEquals(corePasskeysEnabled({ enabled: false }), false);
  assertEquals(corePasskeysEnabled({ enabled: true }), true);
});

test("core account security is not mounted by a Plugin slot or native claim", async () => {
  const panel = await readFile(
    new URL("./ProductAccountSecurity.tsx", import.meta.url), "utf8",
  );
  assert(panel.includes("corePasskeysEnabled(passkeys)"));
  assert(panel.includes("<ProductPasskeysPanel />"));
  for (
    const forbidden of [
      "PluginSlot",
      "hostPlugins",
      "native_capabilities",
      "@cowboy/plugin-api",
    ]
  ) {
    assertEquals(panel.includes(forbidden), false, forbidden);
  }
  const host = await readFile(
    new URL("../pluginHost.ts", import.meta.url), "utf8",
  );
  assertEquals(host.includes('"login-password-v1"'), false);
  assertEquals(host.includes('"account-passkeys-v1"'), false);
  assertEquals(host.includes("ProductPasskeysPanel"), false);
  assertEquals(
    host.includes('contextRecord(context)?.kind !== "password"'),
    false,
  );
  const login = await readFile(
    new URL("./ProductLoginPage.tsx", import.meta.url), "utf8",
  );
  assert(login.includes("busy || passwordMode === null"));
  assert(login.includes('loginContext?.kind === "password"'));
  assert(login.includes("pluginId={selectedProvider.id}"));
  assertEquals(login.includes("passwordLoginFields(hostPlugins)"), false);
});
