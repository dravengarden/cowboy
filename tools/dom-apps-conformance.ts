/** Admission suite for full-DOM apps (docs/widget-apps-dom.md §7, §8).
 *
 * The shell and its headers come from the real `/app-shell.html` route of a
 * disposable Controller built from this tree. A dedicated Chrome is launched
 * on the Mac for the run (its own profile, DevTools port and network log;
 * never the person's or the bridge's browser). The production frame host
 * (`startDomApp`) runs a rendering app and an egress-probe app. Egress is
 * observed outside the page: DevTools interception of every URL, Chrome's
 * network log for connection hints and DNS, and STUN/TURN listeners.
 *
 * Modes: `policy` must show no egress and an out-of-process frame; `control`
 * serves the same shell without its security headers and must show egress on
 * every channel; `no-isolation` disables sandboxed-frame isolation and must be
 * detected as not isolated.
 *
 * Usage: bun tools/dom-apps-conformance.ts <controller> <web-root> <policy|control|no-isolation>
 *   COWBOY_SIM_MAC_HOST selects the Mac's SSH alias (default macbook-air).
 */
import { join } from "node:path";
import { tmpdir } from "node:os";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { Command } from "./lib/command.ts";

const [controller, webRoot, mode] = process.argv.slice(2);
if (!controller || !webRoot || !["policy", "control", "no-isolation"].includes(mode ?? "")) {
  throw new Error(
    "usage: dom-apps-conformance.ts <controller> <web-root> <policy|control|no-isolation>",
  );
}
const host = process.env["COWBOY_SIM_MAC_HOST"] ?? "macbook-air";
if (!/^[A-Za-z0-9_.-]+$/.test(host)) throw new Error("invalid SSH host alias");
const run = crypto.randomUUID().replaceAll("-", "").slice(0, 12);
const ORIGIN = "https://cowboy.test";
// The Controller trusts the forwarded scheme from loopback, as behind Caddy.
const FORWARDED = { "x-forwarded-proto": "https", "x-forwarded-host": "cowboy.test" };

async function exec(
  command: string,
  args: string[],
  env?: Record<string, string>,
): Promise<string> {
  const output = await new Command(command, {
    args,
    env,
    stdout: "piped",
    stderr: "inherit",
  }).output();
  if (!output.success) throw new Error(`${command} ${args[0] ?? ""} failed`);
  return new TextDecoder().decode(output.stdout).trim();
}
const control = join(tmpdir(), `cowboy-dom-${process.pid}`);
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

// 1. The fixture page bundle and the real shell response.
const fixtureDir = await mkdtemp(join(tmpdir(), "cowboy-dom-fixture-"));
let fixture: Uint8Array;
let shell: { body: string; headers: [string, string][] };
try {
  await exec("node", ["tools/idb-browser-bundle.mjs", fixtureDir, "dom-apps"]);
  fixture = await readFile(join(fixtureDir, "fixture.js"));
} finally {
  await rm(fixtureDir, { recursive: true });
}
{
  const root = await mkdtemp(join(tmpdir(), "cowboy-dom-controller-"));
  const port = 39_000 + Math.floor(Math.random() * 900);
  const child = Bun.spawn(
    [
      controller,
      "serve",
      "--bind",
      `127.0.0.1:${port}`,
      "--data-dir",
      join(root, "data"),
      "--web-root",
      webRoot,
      "--workspace-root",
      root,
    ],
    {
      env: {
        ...process.env,
        COWBOY_DATABASE_URL: `sqlite://${join(root, "cowboy.sqlite")}?mode=rwc`,
        COWBOY_PUBLIC_ORIGIN: ORIGIN,
        RUST_LOG: "warn",
      },
      stdout: "ignore",
      stderr: "ignore",
    },
  );
  try {
    let response: Response | null = null;
    for (let attempt = 0; attempt < 120 && !response; attempt += 1) {
      response = await fetch(`http://127.0.0.1:${port}/app-shell.html`, {
        headers: FORWARDED,
      }).catch(() => null);
      if (!response) await Bun.sleep(250);
    }
    if (response?.status !== 200)
      throw new Error(`the Controller did not serve /app-shell.html (${response?.status})`);
    const query = await fetch(`http://127.0.0.1:${port}/app-shell.html?leak=1`, {
      headers: FORWARDED,
    });
    if (query.status !== 404) throw new Error("the shell route answered a query string");
    shell = { body: await response.text(), headers: [...response.headers.entries()] };
  } finally {
    child.kill();
    await child.exited;
    // The Controller installs its bundled plugins read-only.
    await exec("chmod", ["-R", "u+w", root]).catch(() => "");
    await rm(root, { recursive: true, force: true });
  }
}
const SECURITY = new Set(["content-security-policy", "connection-allowlist", "permissions-policy"]);
const shellHeaders = shell.headers
  .filter(([name]) =>
    [
      "content-type",
      "cache-control",
      "referrer-policy",
      "x-content-type-options",
      "cross-origin-resource-policy",
      ...SECURITY,
    ].includes(name),
  )
  .filter(([name]) => mode !== "control" || !SECURITY.has(name))
  .map(([name, value]) => ({ name, value }));

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
const remote = await ssh("mktemp -d -t cowboy-dom-conformance");
if (!/^\/[A-Za-z0-9_./-]+$/.test(remote)) throw new Error("unexpected remote directory");
const devtools = 9300 + Math.floor(Math.random() * 90);
const report: Record<string, unknown> = { mode, run };
let sinkPid = "";
let chromePid = "";
try {
  const upload = Bun.spawn(["ssh", ...SSH, "--", host, `/bin/sh -ec 'cat > ${remote}/sink.py'`], {
    stdin: "pipe",
  });
  upload.stdin.write(SINK);
  upload.stdin.end();
  await upload.exited;
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
    ...(mode === "no-isolation" ? ["--disable-features=IsolateSandboxedIframes"] : []),
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
  let child = "";
  const page = `<!doctype html><meta charset="utf-8"><body><script type="module" src="/boot.js"></script>`;
  const boot = `import { installDomAppsConformance } from "/fixture.js"; installDomAppsConformance(); window.__ready = true;`;
  const parentCsp =
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; frame-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
  const answer = (url: URL): [number, { name: string; value: string }[], string | Uint8Array] => {
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
    if (url.origin === ORIGIN && url.pathname === "/app-shell.html" && !url.search)
      return [200, shellHeaders, shell.body];
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
      if (data.params.targetInfo.type === "iframe") child = session;
      void (async () => {
        await raw(
          "Target.setAutoAttach",
          { autoAttach: true, waitForDebuggerOnStart: true, flatten: true },
          session,
        );
        await raw("Fetch.enable", { patterns: [{ urlPattern: "*" }] }, session);
        await raw("Runtime.runIfWaitingForDebugger", {}, session);
      })();
      return;
    }
    if (data.method === "Fetch.requestPaused") {
      const [status, headers, body] = answer(new URL(data.params.request.url));
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
  await raw("Fetch.enable", { patterns: [{ urlPattern: "*" }] }, session);
  await raw("Page.navigate", { url: `${ORIGIN}/` }, session);
  const evaluate = async (expression: string, target = session): Promise<any> =>
    (await raw("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, target))
      .result?.value;
  const until = async (test: (log: string[]) => boolean, ms: number): Promise<string[]> => {
    const end = Date.now() + ms;
    let log: string[] = [];
    while (Date.now() < end) {
      log = (await evaluate("window.__domApps?.log ?? []")) ?? [];
      if (test(log)) break;
      await Bun.sleep(300);
    }
    return log;
  };
  for (let attempt = 0; attempt < 40 && !(await evaluate("window.__ready === true")); attempt += 1)
    await Bun.sleep(250);
  await evaluate(`window.__domApps.start("boot", ${JSON.stringify(run)})`);
  let log = await until(
    (l) => l.some((x) => x.startsWith("send:{")) || l.some((x) => x.startsWith("stopped:")),
    30_000,
  );
  report["boot"] = log.find((x) => x.startsWith("send:{"))?.slice(5) ?? log;
  if (child) {
    await evaluate(`document.getElementById("b").click()`, child);
    log = await until((l) => l.includes("send:clicked:1"), 5_000);
    report["clicked"] = log.includes("send:clicked:1");
  }
  report["isolated"] = frames.some((frame) => frame.startsWith("iframe"));
  // The product row (§9 step 3): Run, confirmation, Stop, ownership, view.
  if (mode === "policy") report["product"] = await evaluate("window.__domApps.product()");
  await evaluate(`window.__domApps.start("probe", ${JSON.stringify(run)})`);
  log = await until((l) => l.filter((x) => x.startsWith("send:{")).length >= 2, 40_000);
  report["probes"] = log.filter((x) => x.startsWith("send:{"))[1]?.slice(5) ?? log;
  await Bun.sleep(3000);
  report["log"] = (await evaluate("window.__domApps.log")).filter(
    (x: string) => !x.startsWith("send:{"),
  );
  report["egress"] = egress.filter((url) => url.includes(run));
  report["frames"] = frames;
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
  if (/^\d+$/.test(sinkPid))
    await ssh(`kill ${sinkPid} 2>/dev/null || true`).catch(() => undefined);
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
const parse = (value: unknown): Record<string, unknown> | null => {
  try {
    return typeof value === "string" ? (JSON.parse(value) as Record<string, unknown>) : null;
  } catch {
    return null;
  }
};
const boot = parse(report["boot"]) ?? {};
const probes = parse(report["probes"]);
const egress = report["egress"] as string[];
const netlog = report["netlogMentions"] as number;
const stun = report["stunTurn"] as number;
const observed = Number.isFinite(netlog) && Number.isFinite(stun);
const probed = probes !== null && PROBES.every((name) => typeof probes[name] === "string");
const rendered =
  boot["title"] === "Hello DOM" &&
  boot["pixel"] === "34,68,255,255" &&
  boot["image"] === 4 &&
  report["clicked"] === true;
const product =
  ((report["product"] as { passed?: unknown[] } | undefined)?.passed?.length ?? 0) === 4;
const verdict =
  !observed || !probed
    ? false
    : mode === "policy"
      ? rendered &&
        product &&
        egress.length === 0 &&
        netlog === 0 &&
        stun === 0 &&
        report["isolated"] === true
      : mode === "control"
        ? CONTROL_CHANNELS.every((channel) =>
            egress.some((url) => url.includes(`//${channel}-${run}.`)),
          ) &&
          netlog > 0 &&
          stun > 0
        : report["isolated"] === false;
report["observed"] = observed;
report["probed"] = probed;
report["passed"] = verdict;
console.log(JSON.stringify(report, null, 2));
if (!verdict) process.exit(1);
