// Executed as an async function body by the opt-in Simulator bridge, and with
// explicit fake browser bindings by native-shell-probe_test.ts. No credentials,
// login actions, clipboard reads, or navigation overrides are used here.
async function probeCowboyNativeShell(remote) {
  const tests = [];
  const check = (name, value) => {
    if (!value) throw Error(name);
    tests.push(name);
  };
  if (remote) {
    check(
      "remote Cowboy origin",
      location.origin === "https://cowboy.stormbird.xyz",
    );
    check("remote document ready", document.readyState === "complete");
    const heading = document.querySelector("form h1");
    check(
      "remote sign-in UI rendered",
      heading?.textContent?.trim() === "Sign in" &&
        heading.getBoundingClientRect().height > 0,
    );
  }
  check("native keyboard shell", window.__cowboyNativeShell === true);
  check(
    "Tauri IPC present",
    typeof window.__TAURI__?.core?.invoke === "function",
  );
  check(
    "native tweaks present",
    typeof window.__cowboySelectionHaptic === "function" &&
      typeof window.__cowboyReadClipboard === "function" &&
      window.__cowboyAuthenticationBrowserBridgeVersion === 2,
  );
  const host = window.__COWBOY_NATIVE_PLUGIN_HOST;
  check(
    "immutable Plugin ABI coexists",
    host?.version === "1.0.0" && Object.isFrozen(host) &&
      Object.isFrozen(host.capabilities),
  );
  let denied = false;
  try {
    await host.invoke("unknown", {});
  } catch {
    denied = true;
  }
  check("unknown Plugin capability rejected", denied);
  const passkeys = await host.invoke("webauthn", {
    action: "capabilities",
    rp_id: "cowboy.stormbird.xyz",
  });
  check(
    "unentitled shell fails closed",
    passkeys.ok === true && passkeys.available === false,
  );
  async function openerDenied(url) {
    try {
      await window.__TAURI__.core.invoke("plugin:opener|open_url", { url });
      return false;
    } catch (error) {
      // Do not count a missing plugin/command as successful scope enforcement.
      return String(error) === "Not allowed to open url " + url;
    }
  }
  check(
    "opener rejects local files",
    await openerDenied("file:///cowboy-conformance-must-not-open"),
  );
  if (remote) {
    const response = await fetch("/api/auth/status", {
      method: "GET",
      credentials: "omit",
      cache: "no-store",
      redirect: "error",
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(8000),
    });
    check(
      "read-only auth status is JSON",
      response.status === 200 &&
        response.headers.get("content-type")?.includes("application/json"),
    );
    const status = await response.json();
    check(
      "auth status contract",
      typeof status?.registration?.enabled === "boolean" &&
        ["disabled", "token", "open"].includes(status.registration.mode) &&
        typeof status.registration.accepts_registration === "boolean",
    );
    check("remote remains logged out", status.me == null);
    // A positive invocation distinguishes a working remote capability from
    // IPC merely existing while all remote commands are disabled.
    await window.__TAURI__.core.invoke("plugin:haptics|selection_feedback");
    check("remote haptics IPC allowed", true);
    check(
      "remote cannot open app settings",
      await openerDenied("app-settings:"),
    );
  }
  return {
    tests,
    phase: remote ? "remote-logged-out" : "shell",
    origin: location.origin,
    user_agent: navigator.userAgent,
  };
}
