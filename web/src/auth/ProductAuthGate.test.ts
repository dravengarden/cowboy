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
    "ProductDevicesPanel.tsx",
    "DeviceAuthorizationPage.tsx",
    "ProductPasskeysPanel.tsx",
    "ProductRecentAuthSheet.tsx",
    "productReauthMethods.ts",
    "ProductSessionGuard.tsx",
    "productSessionAlertHost.ts",
    "sessionSchedule.ts",
    "PasskeyReauthLock.tsx",
    "passkeyReauthSchedule.ts",
    "passkeyBrowser.ts",
    "passkeyNative.ts",
    "passkeyTransport.ts",
    "passkeyExternalPage.ts",
    "passkeyFlow.ts",
    "pkce.ts",
    "nativeOidcFlow.ts",
    "recentAuth.ts",
    "idleLock.ts",
    "useIdlePasskeyLock.ts",
  ];
  const chunks = await Promise.all(
    names.map((name) => Deno.readTextFile(new URL(name, authDir))),
  );
  return chunks.join("\n");
}

Deno.test("auth package never imports store.ts or opens the product WebSocket", async () => {
  const source = await readAuthSources();
  assertEquals(source.includes('from "../store"'), false);
  assertEquals(source.includes('from "../store.ts"'), false);
  assertEquals(source.includes('from "./store"'), false);
  assertEquals(source.includes('from "../store.ts"'), false);
  assert(source.includes("nativeOidcEventsPath"));
  assert(source.includes("new WebSocket"));
  assertEquals(
    source.includes("new WebSocket(`${proto}//${globalThis.location.host}/ws"),
    false,
  );
  assertEquals(source.includes('"/ws"'), false);
});

Deno.test("ProductAuthGate wraps DesktopApp and MobileApp in main.tsx", async () => {
  const main = await Deno.readTextFile(new URL("main.tsx", webSrc));
  const app = await Deno.readTextFile(new URL("App.tsx", webSrc));
  assert(main.includes("ProductAuthGate"));
  assert(main.includes("<ProductAuthGate>"));
  assert(main.includes("DeviceAuthorizationRoute"));
  assert(main.includes("captureDeviceAuthorizationFromLocation"));
  assert(main.includes("MachineSetupGate"));
  assert(main.includes("<DesktopApp"));
  assert(main.includes("<MobileApp"));
  assertEquals(app.includes("ProductAuthGate"), false);
  assertEquals(app.includes("/api/auth/status"), false);
});

Deno.test("system Safari Passkey page is isolated from the cached app shell", async () => {
  const page = await Deno.readTextFile(
    new URL("../../passkey.html", import.meta.url),
  );
  const externalPage = await Deno.readTextFile(
    new URL("passkeyExternalPage.ts", authDir),
  );
  const worker = await Deno.readTextFile(
    new URL("../../public/sw.js", import.meta.url),
  );
  assert(page.includes('name="referrer" content="no-referrer"'));
  assert(page.includes("default-src 'none'"));
  assert(page.includes('src="/src/auth/passkeyExternalPage.ts"'));
  assert(page.includes('id="continue"'));
  assert(page.includes('id="cancel"'));
  assert(
    externalPage.includes(
      'continueButton?.addEventListener("click", () => void performPasskey())',
    ),
  );
  assert(
    externalPage.includes(
      'if (nativeCallback && options.action === "assert")',
    ),
  );
  assertEquals(externalPage.match(/performPasskey\(\)/g)?.length, 3);
  assert(
    externalPage.includes(
      'if (nativeCallback && ceremony.action === "assert")',
    ),
  );
  assert(externalPage.includes("history.replaceState"));
  assert(externalPage.includes("externalPasskeyApi.complete(transactionId"));
  const cancelHandler = externalPage.slice(
    externalPage.indexOf('cancelButton?.addEventListener("click"'),
    externalPage.indexOf("history.replaceState"),
  );
  assert(cancelHandler.includes("externalPasskeyApi.fail(transactionId)"));
  assertEquals(externalPage.includes('addEventListener("pagehide"'), false);
  assertEquals(externalPage.includes("navigator.sendBeacon"), false);
  assert(worker.includes('url.pathname === "/passkey.html"'));
  assert(worker.includes("event.respondWith(fetch(request))"));
});

Deno.test("official Apple shells isolate their WebAuthn association", async () => {
  const association = JSON.parse(
    await Deno.readTextFile(
      new URL(
        "../../public/.well-known/apple-app-site-association",
        import.meta.url,
      ),
    ),
  ) as { webcredentials?: { apps?: unknown } };
  const defaultMacConfig = JSON.parse(
    await Deno.readTextFile(
      new URL(
        "../../../apps/native-shell/tauri/tauri.macos.conf.json",
        import.meta.url,
      ),
    ),
  ) as { bundle?: { macOS?: Record<string, unknown> } };
  const passkeyMacConfig = JSON.parse(
    await Deno.readTextFile(
      new URL(
        "../../../apps/native-shell/tauri/tauri.macos.passkeys.conf.json",
        import.meta.url,
      ),
    ),
  ) as { bundle?: { macOS?: Record<string, unknown> } };

  assertEquals(association.webcredentials?.apps, [
    "9Z95WKM9DT.top.thundersparrow.cowboy",
  ]);
  assertEquals(defaultMacConfig.bundle?.macOS?.signingIdentity, "-");
  assertEquals(defaultMacConfig.bundle?.macOS?.entitlements, undefined);
  assertEquals(
    passkeyMacConfig.bundle?.macOS?.entitlements,
    "./Entitlements.plist",
  );
  assertEquals(passkeyMacConfig.bundle?.macOS?.signingIdentity, undefined);
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
  assert(login.includes("nativeOidcFlowSupported"));
  assert(login.includes("runNativeOidc"));
  assert(login.includes("href={useNativeProviderFlow ? undefined"));
  assert(login.includes("resolveProductLoginMethodOrder"));
  assert(login.includes("orderedMethodIds[0]"));
  assert(gate.includes("status.login_method_order"));
  assert(gate.includes("<ConfirmSheet"));
  assertEquals(gate.includes("<Dialog"), false);
  assert(gate.includes("Periodic Passkey verification stays off"));
  assert(gate.includes("passkeyFlowCancelled(reason)"));
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

Deno.test("desktop can manage devices and sign out without importing store", async () => {
  const desktop = await Deno.readTextFile(
    new URL("desktop/DesktopApp.tsx", webSrc),
  );
  const clients = await Deno.readTextFile(
    new URL("ProductDevicesPanel.tsx", authDir),
  );
  const capacity = await Deno.readTextFile(
    new URL("ProductSessionCapacityPanel.tsx", authDir),
  );
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  const store = await Deno.readTextFile(new URL("store.ts", webSrc));
  assert(desktop.includes("useProductAuth"));
  assert(desktop.includes("account.signOut"));
  assert(desktop.includes("account.devices"));
  assert(desktop.includes("account.sessions"));
  assert(desktop.includes("ProductDevicesPanel"));
  assert(desktop.includes("ProductSessionCapacityPanel"));
  assert(capacity.includes('["Authorized clients", inventory'));
  assert(capacity.includes("capacity.authorized_clients_per_user"));
  assert(capacity.includes("Effective server policy"));
  assert(capacity.includes("Automation credentials and their separate client pool are disabled"));
  assert(desktop.includes("CLI & ACP access"));
  assert(clients.includes("Browser cookie sessions and Passkeys"));
  assert(clients.includes("hideWhenEmpty"));
  assert(clients.includes("Browser-approved client credentials"));
  assertEquals(clients.includes("Authorized devices"), false);
  assertEquals(desktop.includes("ProductTokensPanel"), false);
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
  assert(sw.includes('const VERSION = "cowboy-v1624"'));
  const authStart = sw.indexOf('url.pathname.startsWith("/api/auth/")');
  const authBranch = sw.slice(
    authStart,
    sw.indexOf("return;", authStart) + "return;".length,
  );
  assert(authBranch.includes("event.respondWith(fetch(request))"));
  assertEquals(authBranch.includes("caches."), false);
  assertEquals(authBranch.includes("caches.match"), false);
});

Deno.test("Passkey changes recover from an expired recent-auth window", async () => {
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  const panel = await Deno.readTextFile(
    new URL("ProductPasskeysPanel.tsx", authDir),
  );
  const sheet = await Deno.readTextFile(
    new URL("ProductRecentAuthSheet.tsx", authDir),
  );
  const retry = await Deno.readTextFile(new URL("recentAuth.ts", authDir));
  assert(gate.includes("options?: RecentProductAuthOptions"));
  assert(gate.includes("<ProductRecentAuthSheet"));
  assert(panel.includes("retryWithRecentProductAuth"));
  assert(retry.includes("isRecentProductAuthRequired"));
  assert(sheet.includes("Verify it’s you"));
  assert(sheet.includes("authApi.login(me.account, password)"));
  assert(sheet.includes("verifyPasskey"));
  assert(sheet.includes("Waiting for Passkey…"));
  assert(sheet.includes("Passkey verification was cancelled. Try again when ready."));
  assert(sheet.includes("runNativeOidc"));
  assert(sheet.includes("runBrowserOidc"));
  assert(sheet.includes("useProviderHandoff"));
  assert(
    sheet.includes("Identity verified. Your pending change is still here."),
  );
  assert(sheet.includes("selectedProvider.button_label"));
  assert(sheet.includes("orderedLoginMethodIds"));
  const addHandler = panel.slice(
    panel.indexOf("const add ="),
    panel.indexOf("const revoke ="),
  );
  const revokeHandler = panel.slice(
    panel.indexOf("const revoke ="),
    panel.indexOf("const toggle ="),
  );
  assert(addHandler.includes("Passkey setup was cancelled. Nothing changed."));
  assert(addHandler.includes('resumeLabel: "Continue to Passkey"'));
  assert(revokeHandler.includes("Revocation was cancelled."));
});

Deno.test("Passkey names are explicit and the product lock is event-driven", async () => {
  const gate = await Deno.readTextFile(new URL("ProductAuthGate.tsx", authDir));
  const panel = await Deno.readTextFile(
    new URL("ProductPasskeysPanel.tsx", authDir),
  );
  const lock = await Deno.readTextFile(
    new URL("PasskeyReauthLock.tsx", authDir),
  );
  const admin = await Deno.readTextFile(
    new URL("admin/AdminPasskeys.tsx", webSrc),
  );
  const idleLock = await Deno.readTextFile(
    new URL("useIdlePasskeyLock.ts", authDir),
  );
  assertEquals(`${gate}\n${panel}\n${admin}`.includes('"This device"'), false);
  assert(gate.includes('const [nickname, setNickname] = useState("")'));
  assert(panel.includes('const [nickname, setNickname] = useState("")'));
  assert(panel.includes("PASSKEY_REAUTH_INTERVALS"));
  const intervals = await Deno.readTextFile(
    new URL("passkeyIntervals.ts", authDir),
  );
  assert(intervals.includes("Every day · Default"));
  assert(intervals.includes("Every hour"));
  assert(intervals.includes("Every 4 hours"));
  assert(intervals.includes("Every 2 days"));
  assertEquals(intervals.includes("Every 7 days"), false);
  assert(lock.includes("globalThis.setTimeout(arm, delay)"));
  assert(lock.includes('addEventListener("visibilitychange", arm)'));
  assertEquals(lock.includes("setInterval"), false);
  assertEquals(idleLock.includes("setInterval"), false);
  assert(idleLock.includes("globalThis.setTimeout(arm, delay)"));
  assert(lock.includes('backdropFilter: "blur(24px) saturate(65%)"'));
  assert(lock.includes("Unlock with Passkey"));
});

Deno.test("Passkey settings use a progressive, visible mobile account hierarchy", async () => {
  const panel = await Deno.readTextFile(
    new URL("ProductPasskeysPanel.tsx", authDir),
  );
  const account = await Deno.readTextFile(
    new URL("ProductAccountMenu.tsx", authDir),
  );
  const externalPage = await Deno.readTextFile(
    new URL("passkeyExternalPage.ts", authDir),
  );
  assert(panel.includes("Add your first Passkey"));
  assert(panel.includes("Registered Passkeys"));
  assert(panel.includes("Periodic verification"));
  assert(panel.includes("Not set up"));
  assert(panel.includes('variant="outlined"'));
  assertEquals(panel.includes("No passkeys yet."), false);
  assert(account.includes("Sign out on this device"));
  assert(account.includes('variant="outlined"'));
  assert(account.includes("Running agents keep"));
  assert(account.includes("retryWithRecentProductAuth"));
  assert(account.includes("reauthenticate"));
  const gate = await Deno.readTextFile(
    new URL("ProductAuthGate.tsx", authDir),
  );
  assert(gate.includes("isRecentProductAuthRequired(reason)"));
  assert(externalPage.includes("Tap Done to return to Cowboy"));
  assert(externalPage.includes("cowboy-passkey://complete"));
  assert(externalPage.includes('finishNative("cancelled")'));
});

Deno.test("session reauthentication is pushed and stays compact until required", async () => {
  const guard = await Deno.readTextFile(
    new URL("ProductSessionGuard.tsx", authDir),
  );
  const sheet = await Deno.readTextFile(
    new URL("ProductRecentAuthSheet.tsx", authDir),
  );
  const panel = await Deno.readTextFile(
    new URL("ProductPasskeysPanel.tsx", authDir),
  );
  const store = await Deno.readTextFile(new URL("store.ts", webSrc));
  const events = await Deno.readTextFile(
    new URL("productAuthEvents.ts", webSrc),
  );

  assert(guard.includes("useSurfaceProfile"));
  assert(guard.includes("safe-area-inset-top"));
  assert(guard.includes('mobile ? { "&&": { minHeight: 44 } }'));
  assert(
    guard.includes('maxWidth: mobile ? "min(17rem, calc(100vw - 24px))"'),
  );
  assert(guard.includes("right: 12"));
  assert(guard.includes("data-product-session-alert-button"));
  assert(guard.includes("createPortal(reminder, desktopHost)"));
  assert(guard.includes("useSyncExternalStore"));
  assert(guard.includes("subscribeProductSessionAlertHost"));
  assertEquals(guard.includes("MutationObserver"), false);
  assert(guard.includes('data-desktop-topbar-action={!mobile ? "reauth"'));
  assert(guard.includes("FINAL_WARNING_MS"));
  assert(sheet.includes("passkeyAbort.current?.abort"));
  assert(sheet.includes("requestEpoch.current += 1"));
  assertEquals(sheet.match(/requestEpoch\.current !== epoch/g)?.length, 2);
  assertEquals(sheet.includes("disabled={busy} onClick={cancel}"), false);
  assert(guard.includes("locked={required}"));
  assert(guard.includes('data-session-lock-backdrop="true"'));
  assert(guard.includes('backdropFilter: "blur(24px) saturate(65%)"'));
  assert(guard.includes("aria-label={`${title}. Open verification`}"));
  assertEquals(guard.includes("collapsed"), false);
  assertEquals(guard.includes("sessionStorage"), false);
  assertEquals(guard.includes("width: mobile"), false);
  assertEquals(guard.includes('component="button"'), false);
  assertEquals(guard.includes("setInterval"), false);
  assertEquals(guard.includes("fetch("), false);
  assert(sheet.includes("announceProductAuthCookieChanged"));
  assert(sheet.includes('purpose === "primary"'));
  assert(sheet.includes("primary-login deadline is approaching"));
  assert(sheet.includes("primary-login limit has been reached"));
  assert(sheet.includes("same method that started this browser session"));
  assert(sheet.includes("session predates sign-in-method tracking"));
  assert(sheet.includes("Sign out to start a new session"));
  assert(sheet.includes("resolvePrimaryReauthMethods"));
  assert(sheet.includes("scheduled Passkey check is approaching"));
  assert(sheet.includes('autoFocus={!mobile && purpose === "primary"}'));
  assert(panel.includes("Session protection"));
  assert(panel.includes("Service settings"));
  assert(panel.includes("currentSessionProtectionItems"));
  assert(panel.includes("configuredSessionProtectionItems"));
  assert(panel.includes("activity never extends the"));
  assert(panel.includes("Off for this account"));
  assert(panel.includes("Verify this browser"));
  assert(store.includes('{ type: "auth_activity" }'));
  assert(store.includes("PRODUCT_AUTH_COOKIE_CHANGED_EVENT"));
  assert(store.includes('reconnectNow("auth_cookie_changed")'));
  assert(store.includes("productSessionPausedForAuth"));
  assert(store.includes("pauseProductSocketForAuth()"));
  assert(store.includes("resumeProductSocketAfterAuthCookieChange"));
  assertEquals(
    store.includes(
      "event.code === WS_AUTH_REQUIRED_CLOSE_CODE) {\n      logoutProductSession();",
    ),
    false,
  );
  assert(events.includes("cowboy:product-auth-session"));
});
