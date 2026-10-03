// Runs only against the disposable Rust transport fixture, never a real login.
(async () => {
  const raw = fetch.bind(window);
  const device = CowboyDeviceProof;
  const tests = JSON.parse(sessionStorage.getItem("tests") || "[]");
  window.fixtureTests = tests;
  const check = (condition, label) => {
    if (!condition) throw new Error(label);
    tests.push(label);
  };
  const decode = (encoded) => JSON.parse(atob(encoded.replaceAll("-", "+").replaceAll("_", "/")));
  try {
    device.install();
    if (!location.search) {
      const frame = document.createElement("iframe");
      frame.src = "/peer";
      const ready = new Promise((resolve) => { frame.onload = resolve; });
      document.body.append(frame);
      await ready;
      const [a, b] = await Promise.all([
        device.proof("/api/private"), frame.contentWindow.CowboyDeviceProof.proof("/api/private"),
      ]);
      check(decode(a).key === decode(b).key, "concurrent browser realms share one persistent key");
      sessionStorage.setItem("key", decode(a).key);
      const db = await new Promise((resolve, reject) => {
        const request = indexedDB.open("cowboy-device-identity", 1);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const keys = await new Promise((resolve) => {
        const request = db.transaction("keys").objectStore("keys").get("identity");
        request.onsuccess = () => resolve(request.result);
      });
      db.close();
      check(keys.privateKey.extractable === false, "private CryptoKey cannot be exported");
      check((await raw("/api/auth/login", { method: "POST" })).status === 401, "unsigned login rejected before dispatch");
      check((await fetch("/api/auth/login", { method: "POST" })).ok, "WebCrypto login proof verified by Rust");
      check(!document.cookie.includes("cowboy_user"), "account cookie is HttpOnly");
      check((await fetch("/api/private")).ok, "bound cookie and signed request accepted");
      check((await raw("/api/private")).status === 401, "cookie without device proof rejected");
      const proof = await device.proof("/api/private");
      const options = { headers: { "x-cowboy-browser-proof": proof } };
      check((await raw("/api/private", options)).ok, "first proof accepted");
      check((await raw("/api/private", options)).status === 401, "captured proof replay rejected");
      const wrongTarget = await device.proof("/api/private?changed");
      check((await raw("/api/private", { headers: { "x-cowboy-browser-proof": wrongTarget } })).status === 401,
        "signature binds the full request target");
      check((await fetch("/api/private", { headers: { "x-cowboy-browser-proof": proof } })).ok,
        "expired or replayed proof retries with a fresh challenge and nonce");
      const wsProof = await device.proof("/ws");
      await new Promise((resolve, reject) => {
        const ws = new WebSocket(location.origin.replace("https:", "wss:") + "/ws", ["cowboy-fixture", `cowboy-device.${wsProof}`]);
        ws.onmessage = (event) => { try {
          check(event.data === "authenticated" && ws.protocol === "cowboy-fixture", "WSS handshake requires and verifies device proof");
          ws.close(); resolve();
        } catch (error) { reject(error); } };
        ws.onerror = () => reject(new Error("signed WSS failed"));
      });
      await new Promise((resolve, reject) => {
        const ws = new WebSocket(location.origin.replace("https:", "wss:") + "/ws", "cowboy-fixture");
        ws.onopen = () => { ws.close(); reject(new Error("unsigned WSS accepted")); };
        ws.onerror = () => { check(true, "unsigned WSS rejected before upgrade"); resolve(); };
      });
      sessionStorage.setItem("tests", JSON.stringify(tests));
      // Exercise a real top-level form_post callback, including the signing
      // bridge, cookie binding and the redirect back to the same browser key.
      const form = document.createElement("form");
      form.method = "POST";
      form.action = "/api/auth/oidc/callback?code=fixture-only";
      document.body.append(form);
      form.submit();
      return;
    }
    check(decode(await device.proof("/api/private")).key === sessionStorage.getItem("key"), "page reload retains the device key");
    check((await fetch("/api/private")).ok, "OIDC form_post bridge binds its new cookie before returning to the app");
    await raw("/fixture/restart", { method: "POST" });
    check((await fetch("/api/private")).ok, "Controller restart retains bindings and automatically refreshes the proof epoch");
    await raw("/report", { method: "POST", body: JSON.stringify({ ok: true, tests }) });
  } catch (error) {
    await raw("/report", { method: "POST", body: JSON.stringify({ ok: false, tests, error: String(error) }) });
  }
})();
