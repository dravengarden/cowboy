import { assert, assertEquals } from "jsr:@std/assert";

const authDir = new URL(".", import.meta.url);
const webSrc = new URL("../", import.meta.url);

async function readAuthSources(): Promise<string> {
  const names = [
    "authApi.ts",
    "authStatus.ts",
    "ProductAuthGate.tsx",
    "ProductLoginPage.tsx",
    "ProductAccountMenu.tsx",
    "ProductTokensPanel.tsx",
    "ProductPasskeysPanel.tsx",
    "PasskeyReauthLock.tsx",
    "idleLock.ts",
    "useIdlePasskeyLock.ts",
  ];
  const chunks = await Promise.all(
    names.map((name) => Deno.readTextFile(new URL(name, authDir))),
  );
  return chunks.join("\n");
}

Deno.test("auth package never imports store.ts or constructs WebSocket", async () => {
  const source = await readAuthSources();
  assertEquals(source.includes('from "../store"'), false);
  assertEquals(source.includes('from "../store.ts"'), false);
  assertEquals(source.includes('from "./store"'), false);
  assertEquals(source.includes('from "../store.ts"'), false);
  assertEquals(/new\s+WebSocket\b/.test(source), false);
  assertEquals(source.includes("/ws"), false);
});

Deno.test("ProductAuthGate wraps DesktopApp and MobileApp in main.tsx", async () => {
  const main = await Deno.readTextFile(new URL("main.tsx", webSrc));
  const app = await Deno.readTextFile(new URL("App.tsx", webSrc));
  assert(main.includes("ProductAuthGate"));
  assert(main.includes("<ProductAuthGate>"));
  assert(main.includes("MachineSetupGate"));
  assert(main.includes("<DesktopApp"));
  assert(main.includes("<MobileApp"));
  assertEquals(app.includes("ProductAuthGate"), false);
  assertEquals(app.includes("/api/auth/status"), false);
});

Deno.test("login page is product chrome and hides register unless accepted", async () => {
  const login = await Deno.readTextFile(
    new URL("ProductLoginPage.tsx", authDir),
  );
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  assert(login.includes("cowboy"));
  assertEquals(login.includes("Cowboy Admin"), false);
  assertEquals(login.includes("<Paper"), false);
  assert(login.includes("Setup code"));
  assert(login.includes("Create the only user"));
  assert(
    login.includes(
      'severity={passwordScore.acceptable ? "success" : "warning"}',
    ),
  );
  assert(login.includes("Good. This password is strong enough"));
  assertEquals(login.includes("Invite token"), false);
  assert(gate.includes("Controller too old or activating"));
  assert(gate.includes("/admin remains the break-glass"));
  assert(gate.includes("this is not a sign-in problem"));
  assert(gate.includes("shouldMountProductApp"));
  assert(gate.includes("nextReadyStatusAction"));
  assert(gate.includes("deleteProductHistoryCache"));
  assert(login.includes('component="form"'));
  assert(login.includes("setupToken.trim()"));
  assert(login.includes("<Tabs"));
  assert(login.includes("selectedProvider.button_label"));
  assert(login.includes('id: "password", label: "Password"'));
  assert(gate.includes("<ConfirmSheet"));
  assertEquals(gate.includes("<Dialog"), false);
  assert(gate.includes("Automatic session refresh stays off"));
});

Deno.test("logged-out gate never mounts product children or /ws", async () => {
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  const readyBranch = gate.slice(
    gate.indexOf('if (view === "ready" && me)'),
    gate.indexOf('if (view === "login")'),
  );
  assert(readyBranch.includes("{children}"));
  const loginBranch = gate.slice(gate.indexOf('if (view === "login")'));
  assertEquals(loginBranch.includes("{children}"), false);
  assert(loginBranch.includes("ProductLoginPage"));
});

Deno.test("desktop can sign out through authApi without importing store", async () => {
  const desktop = await Deno.readTextFile(
    new URL("desktop/DesktopApp.tsx", webSrc),
  );
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  const store = await Deno.readTextFile(new URL("store.ts", webSrc));
  assert(desktop.includes("useProductAuth"));
  assert(desktop.includes("account.signOut"));
  assert(desktop.includes("account.tokens"));
  assert(desktop.includes("ProductTokensPanel"));
  assertEquals(desktop.includes('from "../store"'), false);
  assert(gate.includes("announceProductSessionEnd"));
  assert(gate.includes("location.reload"));
  assert(gate.includes("generationRef"));
  assert(store.includes("cowboy:product-sign-out"));
  assert(store.includes("abandonProductSocket"));
  assert(store.includes("productSessionAbandoned"));
  assert(store.includes("/api/auth/me"));
  assert(store.includes("classifyMeHandshake"));
  assert(store.includes("4001"));
  assert(store.includes("cowboy:product-auth-lost"));
  assertEquals(store.includes('from "./auth/'), false);
  assert(gate.includes("PRODUCT_AUTH_LOST_EVENT"));
  const authLostHandler = gate.slice(
    gate.indexOf("const onAuthLost"),
    gate.indexOf("globalThis.addEventListener(PRODUCT_AUTH_LOST_EVENT"),
  );
  assert(authLostHandler.includes("void loadStatus()"));
  assertEquals(authLostHandler.includes("authApi.logout()"), false);
  assertEquals(authLostHandler.includes('setView("login")'), false);
});

Deno.test("service worker does not cache /api/auth and bumped VERSION", async () => {
  const sw = await Deno.readTextFile(new URL("../../public/sw.js", authDir));
  assert(sw.includes('const VERSION = "cowboy-v1574"'));
  const authStart = sw.indexOf('url.pathname.startsWith("/api/auth/")');
  const authBranch = sw.slice(
    authStart,
    sw.indexOf("return;", authStart) + "return;".length,
  );
  assert(authBranch.includes("event.respondWith(fetch(request))"));
  assertEquals(authBranch.includes("caches."), false);
  assertEquals(authBranch.includes("caches.match"), false);
});
