import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

const appUrl = new URL("./AdminApp.tsx", import.meta.url);
const apiUrl = new URL("./adminApi.ts", import.meta.url);

Deno.test("admin app exposes the operator surfaces", async () => {
  const source = await Deno.readTextFile(appUrl);
  for (const path of [
    "/admin",
    "/admin/login",
    "/admin/accounts",
    "/admin/permissions",
    "/admin/releases",
    "/admin/sessions",
    "/admin/limits",
  ]) {
    assertStringIncludes(source, path);
  }
});

Deno.test("accounts page grows a product users card with operator default", async () => {
  const source = await Deno.readTextFile(appUrl);
  assertStringIncludes(source, "Product users");
  assertStringIncludes(source, "createProductUser");
  assertStringIncludes(source, "disableProductUser");
  assertStringIncludes(source, "setProductUserPassword");
  assertStringIncludes(source, 'useState<AdminRole>("operator")');
  assertStringIncludes(source, "Create user");
  assertStringIncludes(source, "Set password");
  assertStringIncludes(source, "not the session PWA login");
  assertStringIncludes(source, "/ is login-only until a product user exists");
  assertStringIncludes(source, "{token.uses_count}/{token.uses_allowed ?? \"∞\"}");
  assertEquals(source.includes("auth-mode"), false);
  assertEquals(source.includes("cowboy.auth.mode"), false);
  assertEquals(source.includes("loopback_trust"), false);
  assertEquals(source.includes("hybrid"), false);
  assertEquals(/\bmode\s*=\s*["']lan["']/.test(source), false);
});

Deno.test("adminApi lists creates disables and sets product user passwords", async () => {
  const source = await Deno.readTextFile(apiUrl);
  assertStringIncludes(source, "productUsers");
  assertStringIncludes(source, "createProductUser");
  assertStringIncludes(source, 'role: AdminRole = "operator"');
  assertStringIncludes(source, '"/api/admin/users"');
  assertStringIncludes(source, "/api/admin/users/${id}/disable");
  assertStringIncludes(source, "/api/admin/users/${id}/password");
  assertEquals(source.includes("/api/admin/auth-mode"), false);
  assertEquals(source.includes("cowboy.auth.mode"), false);
  assertEquals(source.includes("loopback_trust"), false);
  assertEquals(source.includes("hybrid"), false);
  assertEquals(/\bmode\s*=\s*["']lan["']/.test(source), false);
});
