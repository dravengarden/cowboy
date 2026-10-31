/** T18 (docs/background-tasks.md §7): live reproduction of a stale "Working"
 * subagent on a disposable local stack.
 *
 * A real `cowboy-controller` on a fresh SQLite database serves the real Web
 * bundle through a loopback TLS proxy (Option 1 requires an HTTPS origin) to a
 * disposable browser context of an already running Chrome. One session holds
 * three running subagents followed by 300 later events. The tab caches the
 * whole window (it reads back to the start), goes offline, the Controller
 * restarts while the three finish in place at their anchors (the durable state
 * its store writer produces) and 20 more events arrive, and the tab comes back
 * online without a reload. The reconnect tail (200 events) overlaps the cached
 * window, so nothing refetches the anchors. The result names each row's chip.
 *
 * Usage: bun tools/background-tasks-live-repro.ts <controller> <web-root> <cdp-endpoint> <out-dir>
 * Never point it at a product database or a signed-in browser profile.
 */
import { Database } from "bun:sqlite";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Command } from "./lib/command.ts";

const [controller, webRoot, endpoint, outDir] = process.argv.slice(2);
if (!controller || !webRoot || !endpoint?.startsWith("http://127.0.0.1:") || !outDir) {
  throw new Error("usage: background-tasks-live-repro.ts <controller> <web-root> <loopback cdp> <out-dir>");
}

const root = await mkdtemp(join(tmpdir(), "cowboy-t18-"));
console.error(`t18 root ${root}`);
const database = join(root, "cowboy.sqlite");
const dataDir = join(root, "data");
const httpPort = 39_000 + Math.floor(Math.random() * 500);
const tlsPort = httpPort + 500;
const origin = `https://127.0.0.1:${String(tlsPort)}`;
console.error(`t18 origin ${origin}`);
const SESSION = "sess-t18-live";
const CHILDREN = ["agent-a", "agent-b", "agent-c"];

async function sh(command: string, args: string[]): Promise<void> {
  const result = await new Command(command, { args, stdout: "null", stderr: "inherit" }).output();
  if (!result.success) throw new Error(`${command} failed`);
}

await sh("openssl", [
  "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
  "-keyout", join(root, "key.pem"), "-out", join(root, "cert.pem"), "-days", "1",
  "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
]);

let child: ReturnType<typeof Bun.spawn> | null = null;
async function startController(): Promise<void> {
  child = Bun.spawn([
    controller!, "serve",
    "--bind", `127.0.0.1:${String(httpPort)}`,
    "--data-dir", dataDir,
    "--web-root", webRoot!,
    "--workspace-root", root,
  ], {
    env: {
      ...process.env,
      COWBOY_DATABASE_URL: `sqlite://${database}?mode=rwc`,
      COWBOY_PUBLIC_ORIGIN: origin,
      RUST_LOG: "warn",
    },
    stdout: Bun.file(join(root, "controller.log")),
    stderr: Bun.file(join(root, "controller.err.log")),
  });
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      const response = await fetch(`http://127.0.0.1:${String(httpPort)}/healthz`);
      if (response.status < 500) return;
    } catch {
      // starting
    }
    await Bun.sleep(250);
  }
  throw new Error("controller did not start");
}

async function stopController(): Promise<void> {
  child?.kill("SIGTERM");
  await child?.exited;
  child = null;
}

// Loopback TLS proxy: the Controller trusts X-Forwarded-Proto from loopback.
const upstream = `127.0.0.1:${String(httpPort)}`;
const proxy = Bun.serve<{ target: WebSocket | null; queue: (string | Uint8Array)[]; url: string; protocols: string[]; cookie: string }>({
  port: tlsPort,
  hostname: "127.0.0.1",
  tls: {
    cert: Bun.file(join(root, "cert.pem")),
    key: Bun.file(join(root, "key.pem")),
  },
  async fetch(request, server) {
    const url = new URL(request.url);
    if (request.headers.get("upgrade")?.toLowerCase() === "websocket") {
      const protocols = (request.headers.get("sec-websocket-protocol") ?? "")
        .split(",").map((value) => value.trim()).filter(Boolean);
      const ok = server.upgrade(request, {
        data: {
          target: null,
          queue: [],
          url: `ws://${upstream}${url.pathname}${url.search}`,
          protocols,
          cookie: request.headers.get("cookie") ?? "",
        },
        headers: protocols[0] ? { "Sec-WebSocket-Protocol": protocols[0] } : {},
      });
      return ok ? undefined : new Response("upgrade failed", { status: 400 });
    }
    const headers = new Headers(request.headers);
    headers.set("x-forwarded-proto", "https");
    headers.set("x-forwarded-for", "127.0.0.1");
    headers.set("x-forwarded-host", `127.0.0.1:${String(tlsPort)}`);
    headers.delete("accept-encoding");
    try {
      const response = await fetch(`http://${upstream}${url.pathname}${url.search}`, {
        method: request.method,
        headers,
        body: request.method === "GET" || request.method === "HEAD" ? null : await request.arrayBuffer(),
        redirect: "manual",
        decompress: false,
      } as RequestInit);
      const out = new Headers();
      for (const [name, value] of response.headers) {
        if (name !== "content-length" && name !== "set-cookie") out.set(name, value);
      }
      for (const cookie of response.headers.getSetCookie()) out.append("set-cookie", cookie);
      return new Response(await response.arrayBuffer(), { status: response.status, headers: out });
    } catch {
      return new Response("upstream unavailable", { status: 502 });
    }
  },
  websocket: {
    open(socket) {
      const target = new WebSocket(socket.data.url, {
        protocols: socket.data.protocols,
        headers: {
          "x-forwarded-proto": "https",
          "x-forwarded-for": "127.0.0.1",
          origin,
          host: `127.0.0.1:${String(tlsPort)}`,
          cookie: socket.data.cookie,
        },
      } as never);
      target.binaryType = "arraybuffer";
      socket.data.target = target;
      target.onopen = () => {
        for (const message of socket.data.queue) target.send(message);
        socket.data.queue = [];
      };
      target.onmessage = (event) => socket.send(event.data as string);
      target.onclose = () => socket.close();
      target.onerror = () => socket.close();
    },
    message(socket, message) {
      const target = socket.data.target;
      if (target?.readyState === WebSocket.OPEN) target.send(message);
      else socket.data.queue.push(message);
    },
    close(socket) {
      socket.data.target?.close();
    },
  },
});

function subagent(child: string, state: string, revision: number): string {
  return JSON.stringify({
    kind: "update",
    update: {
      sessionUpdate: "cowboy_subagent",
      sessionId: child,
      revision,
      title: `Background agent ${child.slice(-1).toUpperCase()}`,
      description: "Explore",
      state,
      stopReason: state === "idle" ? "end_turn" : null,
      cancellable: state === "running",
      prompt: "Investigate one part of the repository.",
      toolCalls: 1,
      messages: 1,
      activity: [{ kind: "tool", id: "t1", title: "Read file", status: "completed", toolKind: "read" }],
      result: state === "idle" ? "Finished." : null,
      reporting: true,
    },
  });
}

function text(seq: number): string {
  return JSON.stringify({
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", messageId: `m${String(seq)}`, content: { type: "text", text: `Line ${String(seq)}. ` } },
  });
}

// --- CDP plumbing (a disposable context; see cdp-fixture.ts) -------------
const version = await (await fetch(`${endpoint}/json/version`)).json() as { Browser: string; webSocketDebuggerUrl: string };
const cdp = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  cdp.onopen = resolve;
  cdp.onerror = reject;
});
let nextId = 0;
const pending = new Map<number, PromiseWithResolvers<any>>();
cdp.onmessage = (message) => {
  const data = JSON.parse(String(message.data));
  if (data.id === undefined) return;
  const waiter = pending.get(data.id);
  pending.delete(data.id);
  if (data.error) waiter?.reject(new Error(JSON.stringify(data.error)));
  else waiter?.resolve(data.result);
};
function raw(method: string, params: object = {}, sessionId?: string): Promise<any> {
  const id = ++nextId;
  const waiter = Promise.withResolvers<any>();
  pending.set(id, waiter);
  cdp.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
  return waiter.promise;
}

const report: Record<string, unknown> = { browser: version.Browser, controller };
const { browserContextId } = await raw("Target.createBrowserContext", { disposeOnDetach: true });
try {
  await startController();
  await stopController();
  // Seed the session the device will cache: three running subagents.
  {
    const db = new Database(database);
    // The agents started early; 300 later events followed while the device
    // was online, so its cached window is contiguous from the first row.
    db.run(
      "INSERT INTO sessions(id, provider, machine_id, cwd, title, origin, status, next_seq) VALUES (?, 'claude-code', 'hawk', ?, 'T18 live reproduction', 'web', 'exited', ?)",
      [SESSION, root, 305],
    );
    db.run("INSERT INTO events(session_id, seq, payload) VALUES (?, 0, ?)", [SESSION, text(0)]);
    CHILDREN.forEach((childId, index) => {
      db.run("INSERT INTO events(session_id, seq, payload) VALUES (?, ?, ?)", [SESSION, index + 1, subagent(childId, "running", 1)]);
    });
    for (let seq = 4; seq < 305; seq += 1) {
      db.run("INSERT INTO events(session_id, seq, payload) VALUES (?, ?, ?)", [SESSION, seq, text(seq)]);
    }
    db.close();
  }
  await startController();
  const setupToken = (await readFile(join(dataDir, "admin-setup.token"), "utf8")).trim();
  const { targetId } = await raw("Target.createTarget", { url: "about:blank", browserContextId, newWindow: true, background: true });
  const { sessionId } = await raw("Target.attachToTarget", { targetId, flatten: true });
  const send = (method: string, params: object = {}) => raw(method, params, sessionId);
  await send("Security.setIgnoreCertificateErrors", { ignore: true });
  await send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await send("Emulation.setDeviceMetricsOverride", { width: 1366, height: 860, deviceScaleFactor: 1, mobile: false });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Network.enable");
  const evaluate = async <T>(expression: string): Promise<T> => {
    const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails).slice(0, 800));
    return result.result.value as T;
  };
  const waitFor = async (expression: string, label: string, timeoutMs = 60_000): Promise<unknown> => {
    const deadline = Date.now() + timeoutMs;
    let failure = "";
    while (Date.now() < deadline) {
      const value = await evaluate<unknown>(expression).catch((error: unknown) => {
        failure = String(error);
        return null;
      });
      if (value) return value;
      await Bun.sleep(250);
    }
    const location = await evaluate<string>("location.href + ' ' + document.title + ' | ' + document.body.innerText.slice(0, 400)").catch(() => "?");
    const shot = await send("Page.captureScreenshot", { format: "png" }).catch(() => null);
    if (shot) await writeFile(join(outDir!, "t18-timeout.png"), Buffer.from(shot.data, "base64"));
    console.error(JSON.stringify(report));
    throw new Error(`timed out: ${label} (${failure}) at ${location}`);
  };
  await send("Page.navigate", { url: `${origin}/` });
  await waitFor("document.readyState !== 'loading' && !!document.body", "first load");
  // Create the instance's only user through the product's own login routes.
  const setup = await evaluate<string>(`(async () => {
    const setup = await fetch('/api/auth/setup', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ token: ${JSON.stringify(setupToken)} }) });
    const register = await fetch('/api/auth/register', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ account: 'fixture', password: 'Fixture-Password-T18-x7Q2' }) });
    const login = await fetch('/api/auth/login', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ account: 'fixture', password: 'Fixture-Password-T18-x7Q2' }) });
    const status = login.status + ' ' + (await (await fetch('/api/auth/status', { cache: 'no-store' })).text()).replace(/.*"me"/, '"me"');
    return setup.status + ':' + register.status + ':' + document.cookie + ':' + status.slice(0, 160) + ':' + JSON.stringify([...register.headers]).slice(0, 400);
  })()`);
  report["setup"] = setup;
  // `?session=` is the product's deep link to one session.
  await send("Page.navigate", { url: `${origin}/?session=${SESSION}` });
  await Bun.sleep(1_000);
  await waitFor("document.readyState !== 'loading' && !!document.body", "app load");
  // Open the seeded session the way a person does: its sidebar row.
  // Read back to the start: the device pages in (and caches) the older rows.
  await waitFor(`(${"(() => { for (const el of document.querySelectorAll('*')) { const style = getComputedStyle(el); if (!/(auto|scroll)/.test(style.overflowY) || el.scrollHeight <= el.clientHeight) continue; el.scrollTop = style.flexDirection === 'column-reverse' ? -el.scrollHeight : 0; } return true; })()"}) && !!document.querySelector('[data-subagent-row]')`, "the seeded session's subagents render");
  // The transcript recycles rows far from the viewport; read the rows where a
  // person would find them, at the start of the conversation.
  const toStart = `(() => { for (const el of document.querySelectorAll('*')) {
    const style = getComputedStyle(el);
    if (!/(auto|scroll)/.test(style.overflowY) || el.scrollHeight <= el.clientHeight) continue;
    el.scrollTop = style.flexDirection === 'column-reverse' ? -el.scrollHeight : 0;
  } return true; })()`;
  const chips = `(() => Object.fromEntries(${JSON.stringify(CHILDREN)}.map((id) => {
    const row = document.querySelector('[data-subagent-row="' + id + '"]');
    return [id, row ? (row.querySelector('[data-task-chip]') ?? row.querySelector('.MuiChip-label'))?.textContent ?? row.textContent : null];
  })))()`;
  await waitFor(`(() => { const c = ${chips}; return Object.values(c).every((v) => v && v.includes('Working')); })()`, "three Working rows");
  report["before_offline"] = await evaluate(chips);
  await evaluate("window.__t18SameDocument = true");
  // The tab goes offline; the three finish while it is away.
  await send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  await Bun.sleep(1_500);
  await stopController();
  {
    const db = new Database(database);
    const next = (db.query("SELECT next_seq FROM sessions WHERE id = ?").get(SESSION) as { next_seq: number }).next_seq;
    let seq = Math.max(next, 10);
    CHILDREN.forEach((childId, index) => {
      // The store writer replaces the anchor row in place (UPSERT at its seq).
      db.run("UPDATE events SET payload = ? WHERE session_id = ? AND seq = ?", [subagent(childId, "idle", 2), SESSION, index + 1]);
    });
    // A few more events while the device is away: the reconnect tail still
    // overlaps the cached window, so no gap is refetched.
    for (let index = 0; index < 20; index += 1) {
      db.run("INSERT INTO events(session_id, seq, payload) VALUES (?, ?, ?)", [SESSION, seq, text(seq)]);
      seq += 1;
    }
    db.run("UPDATE sessions SET next_seq = ? WHERE id = ?", [seq, SESSION]);
    db.close();
  }
  await startController();
  await send("Network.emulateNetworkConditions", { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  // Wait for the reconnect bootstrap (no reload) and the reconciliation.
  const deadline = Date.now() + 45_000;
  let after: Record<string, string | null> = {};
  while (Date.now() < deadline) {
    await evaluate(toStart).catch(() => undefined);
    await Bun.sleep(300);
    after = await evaluate(chips);
    if (Object.values(after).every((value) => value?.includes("Done"))) break;
    await Bun.sleep(500);
  }
  report["after_reconnect"] = after;
  report["reloaded"] = !(await evaluate<boolean>("window.__t18SameDocument === true"));
  report["all_done"] = Object.values(after).every((value) => value?.includes("Done"));
  const shot = await send("Page.captureScreenshot", { format: "png" });
  await writeFile(join(outDir, `t18-${report["all_done"] ? "fixed" : "stale"}.png`), Buffer.from(shot.data, "base64"));
} finally {
  await stopController().catch(() => undefined);
  proxy.stop(true);
  await raw("Target.disposeBrowserContext", { browserContextId }).catch(() => undefined);
  cdp.close();
}
console.log(JSON.stringify(report, null, 2));
