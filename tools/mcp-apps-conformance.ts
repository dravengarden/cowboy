/** Admission suite for signed MCP Apps views (docs/widget-apps-mcp.md §9).
 *
 * The conformance view (`web/src/mcpAppsConformanceView.ts`) is served at
 * `/mcp-app/<token>.html` with the headers the Controller computes for those
 * exact bytes (`export_view_headers_for_admission`). A dedicated Chrome on
 * the Mac runs the production frame host and bridge (`startMcpApp`) in a page
 * with the product's parent `frame-src`; the Controller's lease and call
 * endpoints are substituted in the page. Egress is observed outside the page:
 * DevTools interception of every URL, Chrome's network log and STUN/TURN
 * listeners, as in the phase B suite.
 *
 * Modes: `policy` must complete the MCP Apps sequence, prompt only for the
 * write call, refuse the ungranted call, remove a flooding view and a
 * navigating one, and show no egress from an out-of-process frame;
 * `control` serves the view without its security headers and must show
 * egress on every channel.
 *
 * Usage: bun tools/mcp-apps-conformance.ts <policy|control>
 *   (inside the dev shell: it runs one ignored Controller test with cargo)
 *   COWBOY_SIM_MAC_HOST selects the Mac's SSH alias (default macbook-air).
 */
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { Command } from "./lib/command.ts";
import {
  HOP_TOKEN,
  mcpAppsConformanceView,
  mcpAppsSdkView,
} from "../web/src/mcpAppsConformanceView.ts";

const [mode] = process.argv.slice(2);
if (mode !== "policy" && mode !== "control") {
  throw new Error("usage: mcp-apps-conformance.ts <policy|control>");
}
const host = process.env["COWBOY_SIM_MAC_HOST"] ?? "macbook-air";
if (!/^[A-Za-z0-9_.-]+$/.test(host)) throw new Error("invalid SSH host alias");
const run = crypto.randomUUID().replaceAll("-", "").slice(0, 12);
const ORIGIN = "https://cowboy.test";

async function exec(
  command: string,
  args: string[],
  env?: Record<string, string>,
): Promise<string> {
  const output = await new Command(command, {
    args,
    ...(env ? { env } : {}),
    stdout: "piped",
    stderr: "inherit",
  }).output();
  if (!output.success) throw new Error(`${command} ${args[0] ?? ""} failed`);
  return new TextDecoder().decode(output.stdout).trim();
}
const control = join(tmpdir(), `cowboy-mcp-${process.pid}`);
const SSH = [
  "-o",
  "BatchMode=yes",
  "-o",
  "ConnectTimeout=15",
  "-o",
  "ControlMaster=auto",
  "-o",
  `ControlPath=${control}`,
  "-o",
  "ControlPersist=180",
];
const ssh = (script: string): Promise<string> =>
  exec("ssh", [...SSH, "--", host, `/bin/sh -ec '${script}'`]);

const report: Record<string, unknown> = { mode, run };

// 1. The page bundle, the views and the Controller's headers for their bytes.
// The SDK view bundles the pinned `@modelcontextprotocol/ext-apps` from its
// own lockfile (tools/mcp-apps-sdk), as a signed single-file view ships it.
const SIGNED_VIEW_BYTES = 512 * 1024;
const work = await mkdtemp(join(tmpdir(), "cowboy-mcp-fixture-"));
const views: Record<string, string> = { main: mcpAppsConformanceView(run) };
const viewHeaders: Record<string, { name: string; value: string }[]> = {};
let fixture: Uint8Array;
try {
  await exec("node", ["tools/idb-browser-bundle.mjs", work, "mcp-apps"]);
  fixture = await readFile(join(work, "fixture.js"));
  const sdkDir = join(work, "sdk");
  await exec("mkdir", ["-p", sdkDir]);
  for (const name of ["package.json", "bun.lock", "entry.js"]) {
    await writeFile(join(sdkDir, name), await readFile(join("tools/mcp-apps-sdk", name)));
  }
  await exec("bun", ["install", "--cwd", sdkDir, "--frozen-lockfile", "--ignore-scripts"]);
  await exec("bun", [
    "build",
    join(sdkDir, "entry.js"),
    "--minify",
    "--format",
    "iife",
    "--target",
    "browser",
    "--outfile",
    join(sdkDir, "sdk.js"),
  ]);
  views["sdk"] = mcpAppsSdkView(await readFile(join(sdkDir, "sdk.js"), "utf8"));
  report["sdkViewBytes"] = new TextEncoder().encode(views["sdk"]).length;
  if ((report["sdkViewBytes"] as number) > SIGNED_VIEW_BYTES) {
    throw new Error("the SDK view exceeds the signed view limit");
  }
  for (const [name, html] of Object.entries(views)) {
    await writeFile(join(work, `${name}.html`), html);
    await exec(
      "cargo",
      [
        "test",
        "--quiet",
        "--locked",
        "-p",
        "cowboy-server",
        "--lib",
        "export_view_headers_for_admission",
        "--",
        "--ignored",
        "--exact",
        "server::mcp_bindings::apps::tests::export_view_headers_for_admission",
      ],
      {
        ...(process.env as Record<string, string>),
        COWBOY_MCP_APP_VIEW: join(work, `${name}.html`),
        COWBOY_MCP_APP_HEADERS_OUT: join(work, `${name}.json`),
      },
    );
    viewHeaders[name] = JSON.parse(await readFile(join(work, `${name}.json`), "utf8"));
  }
} finally {
  await rm(work, { recursive: true, force: true });
}
const SECURITY = new Set(["content-security-policy", "connection-allowlist", "permissions-policy"]);
const served = Object.fromEntries(
  Object.entries(viewHeaders).map(([name, headers]) => [
    name,
    headers.filter(({ name: header }) => mode !== "control" || !SECURITY.has(header)),
  ]),
);
if (mode === "policy" && !served["main"]?.some(({ name }) => name === "content-security-policy")) {
  throw new Error("the Controller sent no view CSP");
}
// One fresh token per run, as the Controller issues them; `hop` is never
// issued to a frame and is reached only by a view navigating itself.
const TOKENS = {
  main: "a".repeat(64),
  flood: "b".repeat(64),
  navigate: "c".repeat(64),
  hop: "e".repeat(64),
  blob: "f".repeat(64),
  sdk: "1".repeat(64),
};
const SCENARIOS = (mode === "control" ? ["main"] : Object.keys(TOKENS)) as (keyof typeof TOKENS)[];
const VIEW_OF: Record<string, string> = Object.fromEntries([
  ...Object.entries(TOKENS).map(([scenario, token]) => [
    `/mcp-app/${token}.html`,
    scenario === "sdk" ? "sdk" : "main",
  ]),
  [`/mcp-app/${HOP_TOKEN}.html`, "main"],
]);

// 2. A dedicated Chrome and the STUN/TURN sink on the Mac.
const SINK = `import socket, threading, sys, time
log = open(sys.argv[1], "a", buffering=1)
def udp(p):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", p))
    while True:
        d, a = s.recvfrom(4096); log.write("udp %d\\n" % len(d))
def tcp(p):
    s = socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind(("127.0.0.1", p)); s.listen(16)
    while True:
        c, a = s.accept(); log.write("tcp\\n"); c.close()
threading.Thread(target=udp, args=(3478,), daemon=True).start()
threading.Thread(target=tcp, args=(3478,), daemon=True).start()
while True: time.sleep(3600)
`;
// The view's origin server: TLS on loopback, answering only the tokens, and
// recording the request metadata Chrome sent.
const VIEW_SERVER = `import http.server, ssl, json, sys, threading
root = sys.argv[1]
views = json.load(open(root + "/views.json"))
paths = json.load(open(root + "/paths.json"))
used = set(); lock = threading.Lock()
log = open(root + "/requests.log", "a", buffering=1)
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        dest = self.headers.get("Sec-Fetch-Dest", "?"); site = self.headers.get("Sec-Fetch-Site", "?")
        cookie = "cookie" if self.headers.get("Cookie") else "no-cookie"
        name = paths.get(self.path)
        # The Controller's route: an exact unused token, from a same-origin frame.
        with lock:
            ok = name is not None and dest == "iframe" and site == "same-origin" and self.path not in used
            if ok: used.add(self.path)
        log.write("%s %s %s %s %d\\n" % (self.path[9:13], dest, site, cookie, 200 if ok else 404))
        if not ok:
            self.send_response(404); self.send_header("content-length", "0"); self.end_headers(); return
        body = views[name]["html"].encode()
        self.send_response(200)
        for h in views[name]["headers"]: self.send_header(h["name"], h["value"])
        self.send_header("content-length", str(len(body)))
        self.end_headers(); self.wfile.write(body)
    def log_message(self, *a): pass
s = http.server.ThreadingHTTPServer(("127.0.0.1", 8443), H)
c = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); c.load_cert_chain(root + "/cert.pem", root + "/key.pem")
s.socket = c.wrap_socket(s.socket, server_side=True)
s.serve_forever()
`;
const remote = await ssh("mktemp -d -t cowboy-mcp-conformance");
if (!/^\/[A-Za-z0-9_./-]+$/.test(remote)) throw new Error("unexpected remote directory");
const devtools = 9300 + Math.floor(Math.random() * 90);
let sinkPid = "";
let viewPid = "";
let chromePid = "";
try {
  const upload = Bun.spawn(["ssh", ...SSH, "--", host, `/bin/sh -ec 'cat > ${remote}/sink.py'`], {
    stdin: "pipe",
  });
  upload.stdin.write(SINK);
  upload.stdin.end();
  await upload.exited;
  const put = async (name: string, content: string): Promise<void> => {
    const writer = Bun.spawn(["ssh", ...SSH, "--", host, `/bin/sh -ec 'cat > ${remote}/${name}'`], {
      stdin: "pipe",
    });
    writer.stdin.write(content);
    writer.stdin.end();
    if ((await writer.exited) !== 0) throw new Error(`upload of ${name} failed`);
  };
  await put("view_server.py", VIEW_SERVER);
  await put(
    "views.json",
    JSON.stringify(
      Object.fromEntries(
        Object.entries(views).map(([name, html]) => [name, { html, headers: served[name] }]),
      ),
    ),
  );
  await put("paths.json", JSON.stringify(VIEW_OF));
  await ssh(
    `cd ${remote} && /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=cowboy.test -keyout key.pem -out cert.pem >/dev/null 2>&1`,
  );
  viewPid = await ssh(
    `cd ${remote} && (nohup /usr/bin/python3 view_server.py ${remote} >${remote}/view.log 2>&1 & echo $!)`,
  );
  sinkPid = await ssh(
    `cd ${remote} && (nohup /usr/bin/python3 sink.py ${remote}/sink.log >/dev/null 2>&1 & echo $!)`,
  );
  const flags = [
    "--headless=new",
    `--user-data-dir=${remote}/profile`,
    `--remote-debugging-port=${devtools}`,
    `--log-net-log=${remote}/netlog.json`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-sync",
    // Only the view's own origin resolves, to the Mac's loopback TLS server.
    '--host-resolver-rules="MAP cowboy.test:443 127.0.0.1:8443"',
    "--ignore-certificate-errors",
  ].join(" ");
  chromePid = await ssh(
    `(nohup "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" ${flags} >${remote}/chrome.log 2>&1 & echo $!)`,
  );
  await exec("ssh", [...SSH, "-O", "forward", "-L", `${devtools}:127.0.0.1:${devtools}`, host]);
  let version: { Browser: string; webSocketDebuggerUrl: string } | null = null;
  for (let attempt = 0; attempt < 60 && !version; attempt += 1) {
    version = await fetch(`http://127.0.0.1:${devtools}/json/version`).then(
      (r) => r.json() as Promise<{ Browser: string; webSocketDebuggerUrl: string }>,
      () => null,
    );
    if (!version) await Bun.sleep(500);
  }
  if (!version) throw new Error("the dedicated Chrome did not start");
  report["browser"] = version.Browser;

  // 3. Drive the page.
  const ws = new WebSocket(
    version.webSocketDebuggerUrl.replace(/ws:\/\/[^/]+/, `ws://127.0.0.1:${devtools}`),
  );
  await new Promise((ok, no) => {
    ws.onopen = ok;
    ws.onerror = no;
  });
  let id = 0;
  const waiting = new Map<number, (value: any) => void>();
  const raw = (method: string, params: object = {}, sessionId?: string): Promise<any> =>
    new Promise((ok) => {
      const n = ++id;
      waiting.set(n, ok);
      ws.send(JSON.stringify({ id: n, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  const egress: string[] = [];
  const frames: string[] = [];
  const navigations: string[] = [];
  let pendingNavigation = "";
  const navigationLabel = (url: string): string =>
    url.startsWith("blob:")
      ? "blob"
      : url.includes("/mcp-app/")
        ? (url.split("/mcp-app/")[1] ?? "").slice(0, 4)
        : "other";
  const page = `<!doctype html><meta charset="utf-8"><body><script type="module" src="/boot.js"></script>`;
  const boot = `import { installMcpAppsConformance } from "/fixture.js"; installMcpAppsConformance(); window.__ready = true;`;
  // The product's parent policy: frames only from Cowboy's own origin.
  const parentCsp =
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
  const tokens = new Set(Object.keys(VIEW_OF));
  const answer = (
    url: URL,
  ): [number, { name: string; value: string }[], string | Uint8Array] | null => {
    if (url.origin === ORIGIN && url.pathname === "/")
      return [
        200,
        [
          { name: "content-type", value: "text/html" },
          { name: "content-security-policy", value: parentCsp },
        ],
        page,
      ];
    if (url.origin === ORIGIN && url.pathname === "/boot.js")
      return [200, [{ name: "content-type", value: "text/javascript" }], boot];
    if (url.origin === ORIGIN && url.pathname === "/fixture.js")
      return [200, [{ name: "content-type", value: "text/javascript" }], fixture];
    // The view travels over the network to the Mac's TLS server, so Chrome
    // attaches the real Sec-Fetch metadata the Controller's route checks.
    if (url.origin === ORIGIN && tokens.has(url.pathname) && !url.search) return null;
    egress.push(url.href.slice(0, 120));
    return [204, [], ""];
  };
  ws.onmessage = (event) => {
    const data = JSON.parse(String(event.data));
    if (data.id) {
      waiting.get(data.id)?.(data.result ?? { error: data.error });
      waiting.delete(data.id);
      return;
    }
    if (data.method === "Target.attachedToTarget") {
      const session = data.params.sessionId as string;
      frames.push(
        `${data.params.targetInfo.type} ${String(data.params.targetInfo.url).slice(0, 60)}`,
      );
      void (async () => {
        await raw(
          "Target.setAutoAttach",
          { autoAttach: true, waitForDebuggerOnStart: true, flatten: true },
          session,
        );
        await raw("Page.enable", {}, session);
        await raw("Network.enable", {}, session);
        await raw("Fetch.enable", { patterns: [{ urlPattern: "*" }] }, session);
        await raw("Runtime.runIfWaitingForDebugger", {}, session);
      })();
      return;
    }
    // What became of each frame navigation: loaded, or blocked in the
    // browser before any request (CSP, revoked network access).
    if (data.method === "Page.frameRequestedNavigation") {
      pendingNavigation = navigationLabel(String(data.params.url));
      return;
    }
    if (data.method === "Page.frameNavigated") {
      const url = String(data.params.frame?.url ?? "");
      if (url.includes("/mcp-app/")) navigations.push(`${navigationLabel(url)} loaded`);
      return;
    }
    if (
      data.method === "Network.loadingFailed" &&
      pendingNavigation &&
      data.params.type === "Document"
    ) {
      navigations.push(`${pendingNavigation} ${data.params.errorText}`);
      pendingNavigation = "";
      return;
    }
    if (data.method === "Fetch.requestPaused") {
      const answered = answer(new URL(data.params.request.url));
      if (!answered) {
        void raw("Fetch.continueRequest", { requestId: data.params.requestId }, data.sessionId);
        return;
      }
      const [status, headers, body] = answered;
      void raw(
        "Fetch.fulfillRequest",
        {
          requestId: data.params.requestId,
          responseCode: status,
          responseHeaders: headers,
          body: Buffer.from(body).toString("base64"),
        },
        data.sessionId,
      );
    }
  };
  const { targetId } = await raw("Target.createTarget", { url: "about:blank" });
  const session = (await raw("Target.attachToTarget", { targetId, flatten: true }))
    .sessionId as string;
  await raw(
    "Target.setAutoAttach",
    { autoAttach: true, waitForDebuggerOnStart: true, flatten: true },
    session,
  );
  await raw("Page.enable", {}, session);
  await raw("Network.enable", {}, session);
  await raw("Fetch.enable", { patterns: [{ urlPattern: "*" }] }, session);
  await raw("Page.navigate", { url: `${ORIGIN}/` }, session);
  const evaluate = async (expression: string): Promise<any> =>
    (
      await raw(
        "Runtime.evaluate",
        { expression, returnByValue: true, awaitPromise: true },
        session,
      )
    ).result?.value;
  const until = async (test: (log: string[]) => boolean, ms: number): Promise<string[]> => {
    const end = Date.now() + ms;
    let log: string[] = [];
    while (Date.now() < end) {
      log = (await evaluate("window.__mcpApps?.log ?? []")) ?? [];
      if (test(log)) break;
      await Bun.sleep(300);
    }
    return log;
  };
  for (let attempt = 0; attempt < 40 && !(await evaluate("window.__ready === true")); attempt += 1)
    await Bun.sleep(250);
  const start = (scenario: keyof typeof TOKENS): Promise<unknown> =>
    evaluate(
      `window.__mcpApps.start(${JSON.stringify(scenario)}, ${JSON.stringify(TOKENS[scenario])})`,
    );
  // Each scenario runs until its frame is removed; the SDK view until it
  // reports (it keeps running).
  for (const scenario of SCENARIOS) {
    await start(scenario);
    const done = scenario === "sdk" ? "sdk:message:" : `${scenario}:stopped:`;
    const log = await until((l) => l.some((x) => x.startsWith(done)), 45_000);
    report[scenario] = log
      .filter((x) => x.startsWith(`${scenario}:`))
      .map((x) => x.slice(scenario.length + 1));
  }
  report["isolated"] = frames.some((frame) => frame.startsWith("iframe"));
  await Bun.sleep(3000);
  report["egress"] = egress.filter((url) => url.includes(run));
  report["frames"] = frames;
  report["navigations"] = navigations;
  ws.close();
} finally {
  if (/^\d+$/.test(chromePid))
    await ssh(`kill ${chromePid} 2>/dev/null || true; sleep 2`).catch(() => undefined);
  // A missing or empty log is a failed observation, never "no egress".
  report["netlogMentions"] = Number(
    await ssh(
      `test -s ${remote}/netlog.json && grep -c "\\"type\\"" ${remote}/netlog.json >/dev/null && (grep -o ${run} ${remote}/netlog.json | wc -l | tr -d " ") || echo missing`,
    ).catch(() => "missing"),
  );
  report["stunTurn"] = Number(
    await ssh(
      `test -f ${remote}/sink.log && (wc -l < ${remote}/sink.log | tr -d " ") || echo missing`,
    ).catch(() => "missing"),
  );
  report["viewRequests"] = (
    await ssh(`cat ${remote}/requests.log 2>/dev/null || true`).catch(() => "")
  )
    .split("\n")
    .filter(Boolean);
  if (/^\d+$/.test(sinkPid))
    await ssh(`kill ${sinkPid} 2>/dev/null || true`).catch(() => undefined);
  if (/^\d+$/.test(viewPid))
    await ssh(`kill ${viewPid} 2>/dev/null || true`).catch(() => undefined);
  await ssh(`rm -rf ${remote}`).catch(() => undefined);
  await exec("ssh", [...SSH, "-O", "exit", host]).catch(() => undefined);
}

// 4. Verdict. Every probe must have reported, and in `control` every
// channel must have been observed, so silence under `policy` means blocked.
const PROBES = [
  "fetch",
  "image",
  "preconnect",
  "dns-prefetch",
  "prefetch",
  "rtc",
  "blank",
  "document-write",
  "srcdoc",
  "blob-doc",
  "data-doc",
  "audio-worklet",
  "paint-worklet",
  "worker",
];
const CONTROL_CHANNELS = [
  "fetch",
  "image",
  "prefetch",
  "srcdoc",
  "blobdoc",
  "datadoc",
  "audioworklet",
  "paintworklet",
  "worker",
];
const logOf = (scenario: string): string[] => (report[scenario] as string[] | undefined) ?? [];
const messageOf = (scenario: string): Record<string, any> | null => {
  const text = logOf(scenario).find((x) => x.startsWith("message:"));
  try {
    return text ? (JSON.parse(text.slice(8)) as Record<string, any>) : null;
  } catch {
    return null;
  }
};
const stoppedOf = (scenario: string): string =>
  logOf(scenario).find((x) => x.startsWith("stopped:")) ?? "";
const main = messageOf("main");
const mainLog = logOf("main");
const egress = (report["egress"] as string[] | undefined) ?? [];
const netlog = report["netlogMentions"] as number;
const stun = report["stunTurn"] as number;
const observed = Number.isFinite(netlog) && Number.isFinite(stun);
const probed = main !== null && PROBES.every((name) => typeof main["probes"]?.[name] === "string");
// The MCP Apps sequence through the production bridge: replay after
// initialize, the app projection, no prompt for the read-only call, one
// for the write, refusals, coalesced sizes, a link and a message.
const sequence =
  main !== null &&
  main["protocol"] === "2026-01-26" &&
  main["host"] === "Cowboy" &&
  main["capabilities"] === "logging,message,openLinks,serverTools" &&
  main["input"] === "main" &&
  main["result"] === "status green" &&
  main["list"] === "fixture_status,fixture_write" &&
  main["readonly"] === "fixture_status ok" &&
  main["write"] === "fixture_write ok" &&
  main["ungranted"] === "refused" &&
  main["unknown"] === -32601 &&
  main["link"] === "ok" &&
  mainLog.includes("call:fixture_status:plain") &&
  !mainLog.some((x) => x.startsWith("confirm:fixture_status")) &&
  mainLog.includes('confirm:fixture_write:{"x":1}') &&
  mainLog.includes("call:fixture_write:confirmed") &&
  !mainLog.some((x) => x.includes("fixture_other")) &&
  mainLog.includes("link:https://example.com/docs") &&
  mainLog.some((x) => x.startsWith("height:")) &&
  stoppedOf("main") !== "";
// The pinned SDK's own sequence: its initialize, handlers, server tool calls
// (a prompt only for the write), sustained auto-resize without removal.
const sdk = messageOf("sdk");
const sdkLog = logOf("sdk");
const sdkSequence =
  sdk !== null &&
  sdk["error"] === undefined &&
  sdk["input"] === "sdk" &&
  sdk["result"] === "status green" &&
  sdk["host"] === "Cowboy" &&
  sdk["capabilities"] === "logging,message,openLinks,serverTools" &&
  sdk["readonly"] === "fixture_status ok" &&
  sdk["write"] === "fixture_write ok" &&
  sdk["ungranted"] === "refused" &&
  sdk["link"] === "ok" &&
  !sdkLog.some((x) => x.startsWith("confirm:fixture_status")) &&
  sdkLog.includes('confirm:fixture_write:{"x":2}') &&
  !sdkLog.some((x) => x.includes("fixture_other")) &&
  sdkLog.filter((x) => x.startsWith("height:")).length >= 3 &&
  stoppedOf("sdk") === "";
// A blob document the view navigates to may post before its load: it holds
// nothing the view lacked (no ungranted call; a write still needs the
// person) and its egress is blocked by the inherited policy.
const blobLog = logOf("blob");
const blobWrite = blobLog.includes("call:fixture_write:plain");
const blobContained =
  stoppedOf("blob").includes("tried to navigate") &&
  !blobLog.some((x) => x.includes("fixture_other")) &&
  (!blobWrite || blobLog.includes('confirm:fixture_write:{"blob":1}'));
const removed =
  stoppedOf("flood").includes("too much or unreadable") &&
  stoppedOf("navigate").includes("tried to navigate") &&
  stoppedOf("hop").includes("tried to navigate");
// Every served view came from a same-origin frame without cookies; the hop
// to another view's URL came from the view's opaque origin and was refused.
const requests = (report["viewRequests"] as string[] | undefined) ?? [];
const servedRequests = requests.filter((entry) => entry.endsWith(" 200"));
const requested =
  servedRequests.length === SCENARIOS.length &&
  servedRequests.every((entry) => entry.slice(5) === "iframe same-origin no-cookie 200");
// A view navigating its frame to another view's URL never gets that view:
// the browser blocks the request (the view's revoked network access), or
// the route refuses it as coming from the view's opaque origin.
const navigations = (report["navigations"] as string[] | undefined) ?? [];
const hop = requests.filter((entry) => entry.startsWith("dddd "));
const hopRefused =
  !navigations.includes("dddd loaded") &&
  !hop.some((entry) => entry.endsWith(" 200")) &&
  (navigations.some((entry) => entry.startsWith("dddd net::ERR_")) ||
    (hop.length > 0 && hop.every((entry) => !entry.includes(" same-origin "))));
const verdict =
  !observed || !probed || !requested
    ? false
    : mode === "policy"
      ? sequence &&
        sdkSequence &&
        blobContained &&
        removed &&
        hopRefused &&
        egress.length === 0 &&
        netlog === 0 &&
        stun === 0 &&
        report["isolated"] === true
      : CONTROL_CHANNELS.every((channel) =>
          egress.some((url) => url.includes(`//${channel}-${run}.`)),
        ) &&
        netlog > 0 &&
        stun > 0;
Object.assign(report, {
  observed,
  probed,
  sequence,
  sdkSequence,
  blobContained,
  removed,
  requested,
  hopRefused,
  passed: verdict,
});
console.log(JSON.stringify(report, null, 2));
if (!verdict) process.exit(1);
