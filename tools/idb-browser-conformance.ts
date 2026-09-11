/** Invoked by the loopback-only just recipe, with a pinned browser executable.
 * No normal browser profile, auth, environment or product endpoint is used.
 */
const browser = Deno.args[0];
// Closed fixture selector; this runner never opens the deployed application.
const suite = Deno.args[1] ?? "idb";
if (
  suite !== "idb" && suite !== "provider-ui" && suite !== "provider-management"
) {
  throw new Error("unknown suite");
}
const entry = suite === "idb"
  ? "runIdbBrowserConformance"
  : suite === "provider-ui"
  ? "runProviderUiBrowserConformance"
  : "runProviderManagementBrowserConformance";
if (!browser?.startsWith("/nix/store/") || !browser.endsWith("/bin/firefox")) {
  throw new Error(
    "pass the absolute .#cowboy-idb-test-browser /bin/firefox path",
  );
}
const temporary = await Deno.makeTempDir({ prefix: "cowboy-idb-browser-" });
let child: Deno.ChildProcess | undefined;
let server: Deno.HttpServer<Deno.NetAddr> | undefined;
let deadline: ReturnType<typeof setTimeout> | undefined;
try {
  const bundle = `${temporary}/fixture.js`;
  const built = await new Deno.Command("node", {
    args: ["tools/idb-browser-bundle.mjs", temporary, suite],
    stdout: "null",
    stderr: "inherit",
  }).output();
  if (!built.success) throw new Error("browser fixture bundle failed");
  const script = await Deno.readTextFile(bundle);
  const digest = Array.from(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new TextEncoder().encode(script)),
    ),
  )
    .map((byte) => byte.toString(16).padStart(2, "0")).join("");
  const profile = `${temporary}/profile`;
  await Deno.mkdir(profile);
  const token = crypto.randomUUID();
  const report = Promise.withResolvers<unknown>();
  server = Deno.serve(
    { hostname: "127.0.0.1", port: 0, onListen: () => {} },
    async (request) => {
      const url = new URL(request.url);
      if (request.method === "GET" && url.pathname === "/fixture.js") {
        return new Response(script, {
          headers: { "Content-Type": "text/javascript" },
        });
      }
      if (request.method === "POST" && url.pathname === `/report/${token}`) {
        report.resolve(await request.json());
        return new Response("ok");
      }
      if (request.method === "GET" && url.pathname === `/${token}`) {
        return new Response(
          `<!doctype html><script type="module">
import { ${entry} } from "/fixture.js";
let result;
try { result = { ok: true, tests: await ${entry}() }; }
catch (error) { result = { ok: false, error: String(error) }; }
await fetch("/report/${token}", { method: "POST", body: JSON.stringify(result) });
</script>`,
          {
            headers: {
              "Content-Type": "text/html",
              "Content-Security-Policy":
                "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'",
            },
          },
        );
      }
      return new Response("not found", { status: 404 });
    },
  );
  const environment = {
    MOZ_HEADLESS: "1",
    MOZ_CRASHREPORTER_DISABLE: "1",
    MOZ_NO_REMOTE: "1",
    XDG_CACHE_HOME: `${temporary}/cache`,
    XDG_CONFIG_HOME: `${temporary}/config`,
    XDG_DATA_HOME: `${temporary}/data`,
    XDG_RUNTIME_DIR: temporary,
  };
  const version = await new Deno.Command(browser, {
    args: ["--version"],
    clearEnv: true,
    env: environment,
    stdout: "piped",
    stderr: "null",
  }).output();
  if (!version.success) throw new Error("browser version probe failed");
  child = new Deno.Command(browser, {
    args: [
      "--headless",
      "--no-remote",
      "--new-instance",
      "--profile",
      profile,
      `http://127.0.0.1:${server.addr.port}/${token}`,
    ],
    clearEnv: true,
    env: environment,
    stdout: "null",
    stderr: "null",
  }).spawn();
  deadline = setTimeout(
    () => report.reject(new Error("browser conformance timed out")),
    30_000,
  );
  void child.status.then(() =>
    report.reject(new Error("browser exited before report"))
  );
  const result = await report.promise;
  if (
    typeof result !== "object" || result === null || !("ok" in result) ||
    result.ok !== true ||
    !("tests" in result) || !Array.isArray(result.tests) ||
    result.tests.length !== (suite === "idb" ? 8 : 6) ||
    !result.tests.every((test) => typeof test === "string")
  ) {
    throw new Error(`browser conformance failed: ${JSON.stringify(result)}`);
  }
  console.log(JSON.stringify(
    {
      ok: true,
      suite,
      browser: new TextDecoder().decode(version.stdout).trim(),
      executable: browser,
      fixture_sha256: digest,
      isolation: "fresh profile / private loopback namespace / no credentials",
      tests: result.tests,
      physical_device: "not_checked",
    },
    null,
    2,
  ));
} finally {
  clearTimeout(deadline);
  if (child) {
    try {
      child.kill("SIGKILL");
    } catch { /* already exited */ }
    await child.status;
  }
  await server?.shutdown();
  // Only the exact directory exclusively created by this runner.
  await Deno.remove(temporary, { recursive: true });
}
