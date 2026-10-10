import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const authDir = new URL(".", import.meta.url);
const webSrc = new URL("../", import.meta.url);

async function readAuthSources(): Promise<string> {
  const names = [
    "authApi.ts",
    "authStatus.ts",
    "ProductAuthGate.tsx",
    "ProductLoginPage.tsx",
    "ProductPasskeysPanel.tsx",
    "ProductAccountSecurity.tsx",
    "ProductAccountMenu.tsx",
    "ProductDevicesPanel.tsx",
    "DeviceAuthorizationPage.tsx",
    "deviceAuthorization.ts",
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
    names.map((name) => readFile(new URL(name, authDir), "utf8")),
  );
  return chunks.join("\n");
}

test("auth package never imports store.ts or opens the product WebSocket", async () => {
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

test("ProductAuthGate wraps DesktopApp and MobileApp in main.tsx", async () => {
  const main = await readFile(new URL("main.tsx", webSrc), "utf8");
  const app = await readFile(new URL("App.tsx", webSrc), "utf8");
  assert(main.includes("ProductAuthGate"));
  assert(main.includes("<ProductAuthGate>"));
  assert(main.includes("DeviceAuthorizationRoute"));
  assert(main.includes("captureDeviceAuthorizationFromLocation"));
  assertEquals(main.includes("MachineSetupGate"), false); // Draft workspace needs no Machine.
  assert(main.includes("<DesktopApp"));
  assert(main.includes("<MobileApp"));
  assertEquals(app.includes("ProductAuthGate"), false);
  assertEquals(app.includes("/api/auth/status"), false);
});

test("system Safari Passkey page is isolated from the cached app shell", async () => {
  const page = await readFile(
    new URL("../../passkey.html", import.meta.url), "utf8",
  );
  const externalPage = await readFile(
    new URL("passkeyExternalPage.ts", authDir), "utf8",
  );
  const worker = await readFile(
    new URL("../../public/sw.js", import.meta.url), "utf8",
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

test("official Apple shells isolate their WebAuthn association", async () => {
  const association = JSON.parse(
    await readFile(
      new URL(
        "../../public/.well-known/apple-app-site-association",
        import.meta.url,
      ), "utf8",
    ),
  ) as { webcredentials?: { apps?: unknown } };
  const defaultMacConfig = JSON.parse(
    await readFile(
      new URL(
        "../../../apps/native-shell/tauri/tauri.macos.conf.json",
        import.meta.url,
      ), "utf8",
    ),
  ) as { bundle?: { macOS?: Record<string, unknown> } };
  const passkeyMacConfig = JSON.parse(
    await readFile(
      new URL(
        "../../../apps/native-shell/tauri/tauri.macos.passkeys.conf.json",
        import.meta.url,
      ), "utf8",
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

test("login page is product chrome and hides register unless accepted", async () => {
  const login = await readFile(
    new URL("ProductLoginPage.tsx", authDir), "utf8",
  );
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  assert(login.includes("cowboy"));
  assertEquals(login.includes("Cowboy Admin"), false);
  assertEquals(login.includes("<Paper"), false);
  assert(login.includes("passwordLoginFields"));
  assert(login.includes("fieldLabels.setup"));
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
  assertEquals(login.includes("<Tabs"), false);
  assert(login.includes("<SegmentedTabs"));
  assert(login.includes("fullWidth"));
  assert(login.includes('justifyContent: "flex-start"'));
  assert(login.includes("cowboy-app-icon-192-v10.png"));
  assert(login.includes('textTransform: "none"'));
  assert(login.includes("selectedProvider.button_label"));
  assert(login.includes("nativeOidcFlowSupported"));
  assert(login.includes("runNativeOidc"));
  assert(login.includes("placeholder={null}"));
  assert(login.includes("href={context.native ? undefined"));
  assert(login.includes("resolveProductLoginMethodOrder"));
  assert(login.includes("orderedMethodIds[0]"));
  assert(login.includes("loginMethodLabel"));
  assert(login.includes("hostPlugins"));
  assert(gate.includes("status.login_method_order"));
  assert(gate.includes("status.host_plugins"));
  assert(gate.includes("<ConfirmSheet"));
  assertEquals(gate.includes("<Dialog"), false);
  assert(gate.includes("Periodic Passkey verification stays off"));
  assert(gate.includes("passkeyFlowCancelled(reason)"));
});

test("the unreachable page keeps retrying and acknowledges a tap", async () => {
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  // A repeated `retry`/`activating` decision re-sets the same view, so the
  // poll effect must re-arm on a value that changes after every settled probe.
  assert(gate.includes("setProbeSeq((seq) => seq + 1)"));
  assert(
    gate.includes(
      "}, [view, cachedIdentity, probing, probeSeq, loadStatus]);",
    ),
  );
  // The tap has to change the screen, or a retry that did fire reads as dead.
  assert(gate.includes("setProbing(true)"));
  assert(gate.includes('{probing ? "Retrying…" : label}'));
  assert(gate.includes("disabled={probing}"));
  assert(gate.includes("probing={probing}"));
  // It also restarts the backoff instead of waiting out the 15s cap.
  assert(gate.includes("attemptsRef.current = 0;\n    void loadStatus();"));
});

test("logged-out gate never mounts product children or /ws", async () => {
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  const readyBranch = gate.slice(
    gate.indexOf('if (view === "ready" && me)'),
    gate.indexOf('if (view === "login")'),
  );
  assert(readyBranch.includes("{children}"));
  const loginBranch = gate.slice(gate.indexOf('if (view === "login")'));
  assertEquals(loginBranch.includes("{children}"), false);
  assert(loginBranch.includes("ProductLoginPage"));
});

test("desktop can manage devices and sign out without importing store", async () => {
  const desktop = await readFile(
    new URL("desktop/DesktopApp.tsx", webSrc), "utf8",
  );
  const clients = await readFile(
    new URL("ProductDevicesPanel.tsx", authDir), "utf8",
  );
  const capacity = await readFile(
    new URL("ProductSessionCapacityPanel.tsx", authDir), "utf8",
  );
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  const store = await readFile(new URL("store.ts", webSrc), "utf8");
  assert(desktop.includes("useProductAuth"));
  assert(desktop.includes("account.signOut"));
  assert(desktop.includes("account.devices"));
  assert(desktop.includes("account.sessions"));
  assert(desktop.includes("ProductDevicesPanel"));
  assert(desktop.includes("ProductSessionCapacityPanel"));
  assert(capacity.includes('["Authorized clients", inventory'));
  assert(capacity.includes("capacity.authorized_clients_per_user"));
  assert(capacity.includes("Effective server policy"));
  assert(
    capacity.includes(
      "Automation credentials and their separate client pool are disabled",
    ),
  );
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

test("service worker does not cache /api/auth and bumped VERSION", async () => {
  const sw = await readFile(new URL("../../public/sw.js", authDir), "utf8");
  const version = /const VERSION = "cowboy-v([1-9]\d*)"/.exec(sw);
  assert(version && Number(version[1]) >= 1683);
  const authStart = sw.indexOf('url.pathname.startsWith("/api/auth/")');
  const authBranch = sw.slice(
    authStart,
    sw.indexOf("return;", authStart) + "return;".length,
  );
  assert(authBranch.includes("event.respondWith(fetch(request))"));
  assertEquals(authBranch.includes("caches."), false);
  assertEquals(authBranch.includes("caches.match"), false);
});

test("Passkey changes recover from an expired recent-auth window", async () => {
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  const panel = await readFile(
    new URL("ProductPasskeysPanel.tsx", authDir), "utf8",
  );
  const sheet = await readFile(
    new URL("ProductRecentAuthSheet.tsx", authDir), "utf8",
  );
  const retry = await readFile(new URL("recentAuth.ts", authDir), "utf8");
  assert(gate.includes("options?: RecentProductAuthOptions"));
  assert(gate.includes("<ProductRecentAuthSheet"));
  assert(gate.includes("hostPlugins={hostPlugins}"));
  assert(panel.includes("retryWithRecentProductAuth"));
  assert(panel.includes("reauthenticate"));
  assert(retry.includes("isRecentProductAuthRequired"));
  assert(sheet.includes("Verify it’s you"));
  assert(sheet.includes("hostPlugins"));
  assert(sheet.includes("authApi.login(me.account, password)"));
  assert(sheet.includes("passwordLoginFields"));
  assertEquals(sheet.includes('label="Account"'), false);
  assertEquals(sheet.includes('label="Password"'), false);
  assert(sheet.includes("verifyPasskey"));
  assert(sheet.includes("Waiting for Passkey…"));
  // The wording now depends on whether a human could have answered the prompt
  // (passkeyFlow.ts): an instant close is a lost tap or an empty provider, not
  // a decision. Both cases still come from one source.
  assert(sheet.includes("passkeyCancellationMessage("));
  assert(sheet.includes("passkeyPromptWasUntouched(startedAtMs)"));
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
  assert(addHandler.includes("registerPasskey"));
  assert(panel.includes('resumeLabel: "Continue to Passkey"'));
  assert(revokeHandler.includes("authApi.deletePasskey"));
});

test("Passkey names are explicit and the product lock is event-driven", async () => {
  const gate = await readFile(new URL("ProductAuthGate.tsx", authDir), "utf8");
  const panel = await readFile(
    new URL("ProductPasskeysPanel.tsx", authDir), "utf8",
  );
  const lock = await readFile(
    new URL("PasskeyReauthLock.tsx", authDir), "utf8",
  );
  const admin = await readFile(
    new URL("admin/AdminPasskeys.tsx", webSrc), "utf8",
  );
  const idleLock = await readFile(
    new URL("useIdlePasskeyLock.ts", authDir), "utf8",
  );
  assertEquals(`${gate}\n${panel}\n${admin}`.includes('"This device"'), false);
  assert(gate.includes('const [nickname, setNickname] = useState("")'));
  assert(panel.includes('useState("")'));
  assert(panel.includes("PASSKEY_REAUTH_INTERVALS"));
  const intervals = await readFile(
    new URL("passkeyIntervals.ts", authDir), "utf8",
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

test("Passkey settings use a progressive, visible mobile account hierarchy", async () => {
  const panel = await readFile(
    new URL("ProductPasskeysPanel.tsx", authDir), "utf8",
  );
  const account = await readFile(
    new URL("ProductAccountMenu.tsx", authDir), "utf8",
  );
  const externalPage = await readFile(
    new URL("passkeyExternalPage.ts", authDir), "utf8",
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
  const gate = await readFile(
    new URL("ProductAuthGate.tsx", authDir), "utf8",
  );
  assert(gate.includes("isRecentProductAuthRequired(reason)"));
  assert(externalPage.includes("Tap Done to return to Cowboy"));
  assert(externalPage.includes("cowboy-passkey://complete"));
  assert(externalPage.includes('finishNative("cancelled")'));
});

test("session reauthentication is pushed and stays compact until required", async () => {
  const guard = await readFile(
    new URL("ProductSessionGuard.tsx", authDir), "utf8",
  );
  const sheet = await readFile(
    new URL("ProductRecentAuthSheet.tsx", authDir), "utf8",
  );
  const panel = await readFile(
    new URL("ProductPasskeysPanel.tsx", authDir), "utf8",
  );
  const store = await readFile(new URL("store.ts", webSrc), "utf8");
  const events = await readFile(
    new URL("productAuthEvents.ts", webSrc), "utf8",
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
  assert(sheet.includes("SignInWindowBlockedError"));
  assert(sheet.includes("setWindowBlocked(true)"));
  assert(sheet.includes("Continue in this window"));
  assert(sheet.includes("href={selectedProvider.start_url}"));
  assert(panel.includes("Session protection"));
  assert(panel.includes("Service settings"));
  assert(panel.includes("data-product-passkeys-panel"));
  assert(panel.includes("authApi.listPasskeys"));
  assert(panel.includes("updateMe(next)"));
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
