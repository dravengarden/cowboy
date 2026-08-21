import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

const appUrl = new URL("./AdminApp.tsx", import.meta.url);
const apiUrl = new URL("./adminApi.ts", import.meta.url);
const swUrl = new URL("../../public/sw.js", import.meta.url);

Deno.test("first-run admin sends the operator to product setup on /", async () => {
  const source = await Deno.readTextFile(appUrl);
  assertStringIncludes(source, "Open Cowboy");
  assertStringIncludes(source, "Open / to enter the setup code");
  assertStringIncludes(source, "the only account");
  assertEquals(source.includes("Create admin"), false);
  assertEquals(source.includes("adminApi.setup"), false);
  assertEquals(source.includes("adminApi.bootstrap"), false);
  assertEquals(source.includes("Setup token"), false);
  assertEquals(source.includes("Create owner"), false);
});

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

Deno.test("accounts page is single-user and has no invite or extra-user chrome", async () => {
  const source = await Deno.readTextFile(appUrl);
  assertStringIncludes(source, "This instance is single-user");
  assertStringIncludes(source, "created on / during first-run");
  assertEquals(source.includes("createProductUser"), false);
  assertEquals(source.includes("issueToken"), false);
  assertEquals(source.includes("Create user"), false);
  assertEquals(source.includes("Add operator"), false);
  assertEquals(source.includes("Enable registration"), false);
  assertEquals(source.includes("Invite tokens"), false);
  assertEquals(source.includes("auth-mode"), false);
  assertEquals(source.includes("cowboy.auth.mode"), false);
  assertEquals(source.includes("loopback_trust"), false);
  assertEquals(source.includes("hybrid"), false);
  assertEquals(/\bmode\s*=\s*["']lan["']/.test(source), false);
});

Deno.test("adminApi still lists the fail-closed extra-user routes", async () => {
  const source = await Deno.readTextFile(apiUrl);
  assertStringIncludes(source, "productUsers");
  assertStringIncludes(source, '"/api/admin/users"');
  assertStringIncludes(source, "/api/admin/auth/setup");
  assertStringIncludes(source, "/api/admin/passkeys");
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
