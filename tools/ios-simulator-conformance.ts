/** Run a browser conformance fixture in iOS Safari on a disposable iOS
 * Simulator of a Mac (docs/widget-apps.md §9: WebKit acceptance).
 *
 * `--production-csp` serves the page with the Controller's App CSP (the app
 * sandbox's boot script authorized by its hash, no inline fixture script) and
 * the runtime's wasm as a file, as the product does.
 *
 * The fixture bundle is built here, copied into a fresh temporary directory
 * on the Mac and served there on loopback only; a Simulator created for this
 * run opens it and the page POSTs its result back. The Simulator, server and
 * directory are removed afterwards. No signed-in browser or product endpoint
 * is involved.
 *
 * Usage: bun tools/ios-simulator-conformance.ts <suite> [device-type] [runtime|auto] [--production-csp]
 *   COWBOY_SIM_MAC_HOST selects the Mac's SSH alias (default macbook-air).
 */
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { Command } from "./lib/command.ts";
import { APP_SANDBOX_BOOT } from "../web/src/widgets/apps/appSandboxBoot.ts";

const args = process.argv.slice(2);
const productionCsp = args.includes("--production-csp");
const [suite = "widget-apps", device = "iPhone-17", runtime = "auto"] = args.filter(
  (arg) => !arg.startsWith("--"),
);
const host = process.env["COWBOY_SIM_MAC_HOST"] ?? "macbook-air";
const ENTRIES: Record<string, [string, number]> = {
  "widget-apps": ["runWidgetAppsBrowserConformance", 14],
};
const selected = ENTRIES[suite];
if (!selected) throw new Error("unknown suite");
if (!/^[A-Za-z0-9-]+$/.test(device) || !/^[A-Za-z0-9.-]+$/.test(runtime)) {
  throw new Error("invalid device type or runtime");
}
if (!/^[A-Za-z0-9_.-]+$/.test(host)) throw new Error("invalid SSH host alias");
const [entry, expectedTests] = selected;

async function run(command: string, args: string[], input?: Uint8Array): Promise<string> {
  const child = new Command(command, {
    args,
    stdin: input ? "piped" : "null",
    stdout: "piped",
    stderr: "inherit",
  }).spawn();
  if (input) {
    child.stdin.write(input);
    child.stdin.end();
  }
  const output = await child.output();
  if (!output.success) throw new Error(`${command} ${args[0] ?? ""} failed`);
  return new TextDecoder().decode(output.stdout).trim();
}

// One multiplexed connection carries every step and poll of this run.
const control = join(tmpdir(), `cowboy-ios-${process.pid}`);
const SSH_OPTIONS = [
  "-o",
  "BatchMode=yes",
  "-o",
  "ConnectTimeout=15",
  "-o",
  "ControlMaster=auto",
  "-o",
  `ControlPath=${control}`,
  "-o",
  "ControlPersist=120",
];
const ssh = (script: string, input?: Uint8Array): Promise<string> =>
  run("ssh", [...SSH_OPTIONS, "--", host, `/bin/sh -ec '${script}'`], input);

// The server answers only the fixture files and one report.
const SERVER = `import http.server, json, os, sys
root = sys.argv[1]
token = open(os.path.join(root, "token")).read().strip()
TYPES = {"/": ("index.html", "text/html"), "/fixture.js": ("fixture.js", "text/javascript"),
         "/loader.js": ("loader.js", "text/javascript")}
CSP = open(os.path.join(root, "csp")).read().strip() if os.path.exists(os.path.join(root, "csp")) else ""
for asset in os.listdir(os.path.join(root, "assets")) if os.path.isdir(os.path.join(root, "assets")) else []:
    TYPES["/assets/" + asset] = ("assets/" + asset, "application/wasm" if asset.endswith(".wasm") else "application/octet-stream")
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        sys.stderr.write("%s %s\\n" % (self.command, self.path))
    def do_GET(self):
        name, kind = TYPES.get(self.path, (None, None))
        if name is None:
            self.send_response(404); self.end_headers(); return
        body = open(os.path.join(root, name), "rb").read()
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Cache-Control", "no-store")
        if CSP and name == "index.html":
            self.send_header("Content-Security-Policy", CSP.replace("{HOST}", self.headers.get("Host", "")))
        self.end_headers(); self.wfile.write(body)
    def do_POST(self):
        if self.path != "/report/" + token:
            self.send_response(404); self.end_headers(); return
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        json.loads(body)
        open(os.path.join(root, "result.json.tmp"), "wb").write(body)
        os.rename(os.path.join(root, "result.json.tmp"), os.path.join(root, "result.json"))
        self.send_response(204); self.end_headers()
server = http.server.HTTPServer(("127.0.0.1", 0), Handler)
open(os.path.join(root, "port"), "w").write(str(server.server_port))
server.serve_forever()
`;

const temporary = await mkdtemp(join(tmpdir(), "cowboy-ios-fixture-"));
let script: Uint8Array;
const assets: [string, Uint8Array][] = [];
try {
  const built = await new Command("node", {
    args: ["tools/idb-browser-bundle.mjs", temporary, suite],
    stdout: "null",
    stderr: "inherit",
  }).output();
  if (!built.success) throw new Error("browser fixture bundle failed");
  script = await readFile(`${temporary}/fixture.js`);
  if (productionCsp) {
    // The fixture bundle inlines the runtime's wasm; the product serves it
    // as a file, and the App CSP admits no data: fetch.
    const text = new TextDecoder().decode(script);
    const inline = /data:application\/wasm;base64,([A-Za-z0-9+/=]+)/.exec(text);
    if (!inline?.[1]) throw new Error("the fixture carries no inline runtime wasm");
    assets.push(["runtime.wasm", Buffer.from(inline[1], "base64")]);
    script = new TextEncoder().encode(text.replace(inline[0], "/assets/runtime.wasm"));
  }
} finally {
  await rm(temporary, { recursive: true });
}
const token = crypto.randomUUID();
const loader = `globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
// Engine facts the enablement predicate reads (web/src/widgets/apps/appsEnabled.ts).
const engine = { anchor: CSS.supports("anchor-name: --a"), standalone: navigator.standalone === true, touchPoints: navigator.maxTouchPoints };
let result;
try { const { ${entry} } = await import("/fixture.js"); result = { ok: true, tests: await ${entry}(), userAgent: navigator.userAgent, engine }; }
catch (error) { result = { ok: false, error: String(error), userAgent: navigator.userAgent, engine }; }
await fetch("/report/${token}", { method: "POST", body: JSON.stringify(result) });
`;
const page = `<!doctype html><meta name="viewport" content="width=device-width,initial-scale=1"><script type="module" src="/loader.js"></script>`;
// The Controller's App CSP for this page, verbatim from web_security.rs
// (`{HOST}` is this page's host): the only inline script is the app
// sandbox's boot, authorized by its hash.
const bootHash = createHash("sha256").update(APP_SANDBOX_BOOT).digest("base64");
const csp =
  `default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-${bootHash}'; ` +
  "style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; " +
  "media-src 'self' data: blob:; connect-src 'self' wss://{HOST} ipc: http://ipc.localhost; " +
  "worker-src 'self' blob:; " +
  "object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

const remote = await ssh("mktemp -d -t cowboy-ios-conformance");
if (!/^\/[A-Za-z0-9_./-]+$/.test(remote)) throw new Error("unexpected remote directory");
let simulator = "";
let server = "";
try {
  const local = await mkdtemp(join(tmpdir(), "cowboy-ios-upload-"));
  try {
    await writeFile(join(local, "fixture.js"), script);
    await writeFile(join(local, "index.html"), page);
    await writeFile(join(local, "server.py"), SERVER);
    await writeFile(join(local, "token"), token);
    await writeFile(join(local, "loader.js"), loader);
    const names = ["fixture.js", "index.html", "server.py", "token", "loader.js"];
    if (productionCsp) {
      await writeFile(join(local, "csp"), csp);
      names.push("csp");
    }
    await mkdir(join(local, "assets"));
    for (const [name, body] of assets) await writeFile(join(local, "assets", name), body);
    names.push("assets");
    await run("scp", [
      "-q",
      "-r",
      ...SSH_OPTIONS,
      ...names.map((name) => join(local, name)),
      `${host}:${remote}/`,
    ]);
  } finally {
    await rm(local, { recursive: true });
  }
  server = await ssh(
    `cd ${remote} && (nohup /usr/bin/python3 server.py ${remote} >server.log 2>&1 & echo $!)`,
  );
  const port = await ssh(
    `for i in $(seq 1 50); do test -s ${remote}/port && cat ${remote}/port && exit 0; sleep 0.2; done; exit 1`,
  );
  const runtimeId =
    runtime === "auto"
      ? await ssh(
          `xcrun simctl list runtimes -j | /usr/bin/python3 -c "import json,re,sys; r=[x for x in json.load(sys.stdin)[\\"runtimes\\"] if x.get(\\"isAvailable\\") and re.search(\\"[.]iOS-[0-9-]+$\\", x[\\"identifier\\"])]; r.sort(key=lambda x: tuple(map(int, x[\\"version\\"].split(\\".\\")))); print(r[-1][\\"identifier\\"])"`,
        )
      : `com.apple.CoreSimulator.SimRuntime.${runtime}`;
  simulator = await ssh(
    `xcrun simctl create "Cowboy conformance ${token.slice(0, 8)}" com.apple.CoreSimulator.SimDeviceType.${device} ${runtimeId}`,
  );
  if (!/^[0-9A-F-]{36}$/.test(simulator)) throw new Error("unexpected simulator id");
  await ssh(`xcrun simctl boot ${simulator} && xcrun simctl bootstatus ${simulator} -b >/dev/null`);
  await ssh(`xcrun simctl openurl ${simulator} http://127.0.0.1:${port}/`);
  const deadline = Date.now() + 240_000;
  let reopen = Date.now() + 45_000;
  let raw = "";
  while (Date.now() < deadline) {
    raw = await ssh(`cat ${remote}/result.json 2>/dev/null || true`);
    if (raw) break;
    // A freshly booted Simulator can drop the first URL before Safari is up.
    if (Date.now() > reopen) {
      reopen = Number.POSITIVE_INFINITY;
      const requested = await ssh(`grep -c "GET / " ${remote}/server.log 2>/dev/null || true`);
      if (requested === "0" || requested === "") {
        await ssh(`xcrun simctl openurl ${simulator} http://127.0.0.1:${port}/`);
      }
    }
    await Bun.sleep(2000);
  }
  if (!raw) {
    const log = await ssh(`tail -40 ${remote}/server.log 2>/dev/null || true`);
    throw new Error(`iOS Simulator conformance timed out; requests:\n${log}`);
  }
  const result = JSON.parse(raw) as {
    ok?: boolean;
    tests?: unknown[];
    error?: string;
    userAgent?: string;
  };
  console.log(
    JSON.stringify({ suite, device, runtime: runtimeId, host, productionCsp, ...result }, null, 2),
  );
  if (result.ok !== true || result.tests?.length !== expectedTests) {
    throw new Error(`iOS Simulator conformance failed: ${result.error ?? "wrong test count"}`);
  }
} finally {
  if (simulator) {
    await ssh(
      `xcrun simctl shutdown ${simulator} >/dev/null 2>&1 || true; xcrun simctl delete ${simulator}`,
    ).catch(() => undefined);
  }
  if (/^\d+$/.test(server)) await ssh(`kill ${server} 2>/dev/null || true`).catch(() => undefined);
  await ssh(`rm -rf ${remote}`).catch(() => undefined);
  await run("ssh", [...SSH_OPTIONS, "-O", "exit", host]).catch(() => undefined);
}
