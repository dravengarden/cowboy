/** The signed view the phase C admission run serves (docs/widget-apps-mcp.md
 *  §9). One self-contained HTML file with one inline script, as a signed
 *  release would carry it: it speaks MCP Apps JSON-RPC to its parent
 *  (initialize, tool input and result, tools/list, a read-only and a write
 *  call, an ungranted call, an unknown method, size notifications, a link
 *  and a message), and runs the phase B egress probes from inside the view.
 *  The tool input picks the scenario: `main`, `flood` or `navigate`. Every
 *  probe host carries the run id so the runner can find any attempt. */
/** An unused token the `hop` scenario navigates its frame to: a request from
 *  the view's opaque origin, which the Controller's route must refuse. */
export const HOP_TOKEN = "d".repeat(64);

export function mcpAppsConformanceView(run: string): string {
  if (!/^[a-z0-9]{1,32}$/.test(run)) throw new Error("invalid run id");
  const script = `
const run = "${run}";
const host = (n) => "https://" + n + "-" + run + ".sink.invalid";
const post = (m) => parent.postMessage(m, "*");
let next = 1;
const waiting = new Map();
const request = (method, params) => new Promise((resolve) => {
  const id = next++; waiting.set(id, resolve); post({ jsonrpc: "2.0", id, method, params });
});
const notify = (method, params) => post({ jsonrpc: "2.0", method, params });
let input = null, result = null, init = null, started = false;
addEventListener("message", (event) => {
  if (event.source !== parent) return;
  const m = event.data;
  if (!m || m.jsonrpc !== "2.0") return;
  if (m.method === undefined && waiting.has(m.id)) { waiting.get(m.id)(m); waiting.delete(m.id); return; }
  if (m.method === "ui/notifications/tool-input") input = m.params.arguments;
  else if (m.method === "ui/notifications/tool-result") { result = m.params; if (!started) { started = true; go(); } }
  else if (m.id !== undefined) post({ jsonrpc: "2.0", id: m.id, result: {} });
});
const inner = (tag) => "<script>fetch('" + host(tag) + "/').catch(()=>0);try{const p=new RTCPeerConnection({iceServers:[{urls:'stun:127.0.0.1:3478'}]});p.createDataChannel('x');p.createOffer().then(o=>p.setLocalDescription(o))}catch(e){}<\\/script>";
async function probes() {
  const r = {};
  const t = async (k, f) => { try { r[k] = String(await f()); } catch (e) { r[k] = "threw " + (e && e.name); } };
  const link = (rel, href) => { const l = document.createElement("link"); l.rel = rel; l.href = href; document.head.append(l); return "added"; };
  await t("fetch", () => fetch(host("fetch") + "/").then((x) => x.status));
  await t("image", () => new Promise((res) => { const i = new Image(); i.onload = () => res("load"); i.onerror = () => res("error"); i.src = host("image") + "/x.png"; }));
  await t("preconnect", () => link("preconnect", host("preconnect")));
  await t("dns-prefetch", () => link("dns-prefetch", "https://dnsprefetch-" + run + ".example.com"));
  await t("prefetch", () => link("prefetch", host("prefetch") + "/p"));
  await t("rtc", async () => {
    const p = new RTCPeerConnection({ iceServers: [{ urls: "stun:127.0.0.1:3478" }, { urls: "turn:127.0.0.1:3478?transport=tcp", username: "u", credential: "p" }] });
    p.createDataChannel("x"); await p.setLocalDescription(await p.createOffer()); await new Promise((s) => setTimeout(s, 2500)); return p.iceGatheringState;
  });
  await t("blank", () => { const f = document.createElement("iframe"); document.body.append(f); f.contentWindow.fetch(host("blank") + "/").catch(() => 0); return "executed"; });
  await t("document-write", () => { const f = document.createElement("iframe"); document.body.append(f); f.contentDocument.write(inner("documentwrite")); f.contentDocument.close(); return "written"; });
  await t("srcdoc", () => { const f = document.createElement("iframe"); f.srcdoc = inner("srcdoc"); document.body.append(f); return "added"; });
  await t("blob-doc", () => { const f = document.createElement("iframe"); f.src = URL.createObjectURL(new Blob([inner("blobdoc")], { type: "text/html" })); document.body.append(f); return "added"; });
  await t("data-doc", () => { const f = document.createElement("iframe"); f.src = "data:text/html," + encodeURIComponent(inner("datadoc")); document.body.append(f); return "added"; });
  await t("audio-worklet", () => new AudioContext().audioWorklet.addModule(URL.createObjectURL(new Blob(["import '" + host("audioworklet") + "/m.js';"], { type: "text/javascript" }))).then(() => "loaded"));
  await t("paint-worklet", () => CSS.paintWorklet ? CSS.paintWorklet.addModule(URL.createObjectURL(new Blob(["import '" + host("paintworklet") + "/m.js';"], { type: "text/javascript" }))).then(() => "loaded") : "absent");
  await t("worker", () => { new Worker(URL.createObjectURL(new Blob(["fetch('" + host("worker") + "/')"], { type: "text/javascript" }))); return "constructed"; });
  await new Promise((s) => setTimeout(s, 3000));
  return r;
}
async function go() {
  if (input && input.mode === "flood") { for (let i = 0; i < 1000; i++) notify("ui/notifications/size-changed", { height: i }); return; }
  if (input && input.mode === "navigate") { location.href = host("navigate") + "/"; return; }
  if (input && input.mode === "hop") { location.href = "/mcp-app/${HOP_TOKEN}.html"; return; }
  if (input && input.mode === "blob") {
    // A document the view creates, posting before its load: it may only use
    // what the view already had, under the view's CSP.
    const doc = "<script>parent.postMessage({jsonrpc:'2.0',id:901,method:'tools/call',params:{name:'fixture_other',arguments:{}}},'*');" +
      "parent.postMessage({jsonrpc:'2.0',id:902,method:'tools/call',params:{name:'fixture_write',arguments:{blob:1}}},'*');" +
      "fetch('" + host("blobnav") + "/').catch(()=>0);<\\/script>";
    location.href = URL.createObjectURL(new Blob([doc], { type: "text/html" }));
    return;
  }
  const r = { protocol: init && init.result && init.result.protocolVersion, host: init && init.result && init.result.hostInfo && init.result.hostInfo.name,
    capabilities: init && init.result && Object.keys(init.result.hostCapabilities || {}).sort().join(","),
    input: input && input.mode, result: result && result.content && result.content[0] && result.content[0].text };
  const list = await request("tools/list", {});
  r.list = ((list.result && list.result.tools) || []).map((tool) => tool.name).join(",");
  const ro = await request("tools/call", { name: "fixture_status", arguments: {} });
  r.readonly = ro.result ? ro.result.content[0].text : "error " + (ro.error && ro.error.message);
  const wr = await request("tools/call", { name: "fixture_write", arguments: { x: 1 } });
  r.write = wr.result ? wr.result.content[0].text : "error " + (wr.error && wr.error.message);
  const ng = await request("tools/call", { name: "fixture_other", arguments: {} });
  r.ungranted = ng.error ? "refused" : "allowed";
  const nf = await request("sampling/createMessage", {});
  r.unknown = nf.error ? nf.error.code : "answered";
  for (let h = 100; h <= 300; h += 50) notify("ui/notifications/size-changed", { height: h });
  const lk = await request("ui/open-link", { url: "https://example.com/docs" });
  r.link = lk.error ? "refused" : "ok";
  r.probes = await probes();
  await request("ui/message", { role: "user", content: [{ type: "text", text: JSON.stringify(r) }] });
  location.href = host("navigate") + "/?after=1";
}
request("ui/initialize", { protocolVersion: "2026-01-26", appInfo: { name: "conformance", version: "1" }, appCapabilities: {} })
  .then((reply) => { init = reply; notify("ui/notifications/initialized", {}); });
`;
  return `<!doctype html><meta charset="utf-8"><title>Conformance view</title><p id="t">view</p><script>${script}</script>`;
}

/** The same scenario driven by the pinned `@modelcontextprotocol/ext-apps`
 *  SDK (`tools/mcp-apps-sdk`): its own initialize sequence, tool input and
 *  result handlers, server tool calls, sustained auto-resize, a link and a
 *  message. `sdk` is the bundled IIFE defining `McpAppsSdk`. */
export function mcpAppsSdkView(sdk: string): string {
  if (sdk.includes("</script")) throw new Error("the SDK bundle cannot be inlined");
  const script = `
const { App, PostMessageTransport } = globalThis.McpAppsSdk;
const app = new App({ name: "conformance-sdk", version: "1" }, {}, { autoResize: true });
const r = {};
app.ontoolinput = (params) => { r.input = params.arguments && params.arguments.mode; };
app.ontoolresult = async (params) => {
  r.result = params.content && params.content[0] && params.content[0].text;
  r.host = app.getHostVersion() && app.getHostVersion().name;
  r.capabilities = Object.keys(app.getHostCapabilities() || {}).sort().join(",");
  try {
    const ro = await app.callServerTool({ name: "fixture_status", arguments: {} });
    r.readonly = ro.content[0].text;
    const wr = await app.callServerTool({ name: "fixture_write", arguments: { x: 2 } });
    r.write = wr.content[0].text;
    try { await app.callServerTool({ name: "fixture_other", arguments: {} }); r.ungranted = "allowed"; }
    catch (e) { r.ungranted = "refused"; }
    for (let i = 0; i < 20; i++) {
      const p = document.createElement("p"); p.textContent = "row " + i; document.body.append(p);
      await new Promise((s) => setTimeout(s, 100));
    }
    const link = await app.openLink({ url: "https://example.com/sdk" });
    r.link = link && link.isError ? "refused" : "ok";
  } catch (e) { r.error = String(e && e.message || e).slice(0, 200); }
  await app.sendMessage({ role: "user", content: [{ type: "text", text: JSON.stringify(r) }] });
};
app.connect(new PostMessageTransport(window.parent, window.parent)).then(
  () => { r.connected = true; },
  (e) => { r.connected = String(e); });
`;
  return `<!doctype html><meta charset="utf-8"><title>SDK view</title><script>${sdk}</script><script>${script}</script>`;
}
