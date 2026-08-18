import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

const appUrl = new URL("./AdminApp.tsx", import.meta.url);
const apiUrl = new URL("./adminApi.ts", import.meta.url);
const swUrl = new URL("../../public/sw.js", import.meta.url);

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

Deno.test("product users failures stay on the card and do not hide Accounts", async () => {
  const source = await Deno.readTextFile(appUrl);
  assertStringIncludes(source, "reloadProductUsers");
  assertStringIncludes(source, "setProductError");
  assertStringIncludes(source, "canSetPassword");
  assertStringIncludes(source, "canGrantOwner");
  assertStringIncludes(source, 'auth.role === "owner"');
  assertStringIncludes(
    source,
    "const [next, accountData] = await Promise.all([\n      adminApi.registration(),\n      adminApi.accounts(),\n    ]);",
  );
  assertStringIncludes(source, "const productData = await adminApi.productUsers();");
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

Deno.test("service worker never stores admin navigations as the PWA shell", async () => {
  const source = await Deno.readTextFile(swUrl);
  assertStringIncludes(source, 'url.pathname === "/admin"');
  assertStringIncludes(source, 'url.pathname.startsWith("/admin/")');
  assertStringIncludes(source, 'url.pathname === "/admin.html"');
  assertStringIncludes(source, "event.respondWith(fetch(request))");
  assertStringIncludes(source, 'c.put("/", copy)');
  const adminGuard = source.slice(
    source.indexOf('url.pathname === "/admin"'),
    source.indexOf('c.put("/", copy)'),
  );
  assertStringIncludes(adminGuard, "event.respondWith(fetch(request))");
  assertEquals(adminGuard.includes('c.put("/", copy)'), false);
});
