/** Full-DOM apps (docs/widget-apps-dom.md §7): the production frame host
 *  (`startDomApp`) runs a rendering app and an egress-probe app in the shell
 *  the runner serves at /app-shell.html. Evidence of egress is collected
 *  outside the page (tools/dom-apps-conformance.ts); this page only starts
 *  apps and records what the host saw. */
import { startDomApp } from "./widgets/dom/domAppHost";
import { runDomAppsProductConformance } from "./domAppsProductConformance";

const DOT =
  "iVBORw0KGgoAAAANSUhEUgAAAAQAAAAECAIAAAAmkwkpAAAAEklEQVR4nGP4z8CAB+GTG8HSALfKY52fctdcAAAAAElFTkSuQmCC";

const BOOT_APP = `
import { useState, useEffect, useRef } from "react";
import * as THREE from "three";
import { useSend, images } from "@cowboy/dom";
export default function App({ title }: { title: string }) {
  const [n, setN] = useState(0); const send = useSend(); const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const r = new THREE.WebGLRenderer({ canvas: ref.current!, preserveDrawingBuffer: true });
    const scene = new THREE.Scene(); scene.background = new THREE.Color(0x2244ff);
    const cam = new THREE.PerspectiveCamera(50, 1, 0.1, 10); cam.position.z = 3;
    scene.add(new THREE.Mesh(new THREE.BoxGeometry(), new THREE.MeshNormalMaterial())); r.render(scene, cam);
    const img = document.getElementById("img") as HTMLImageElement;
    const report = () => {
      const t = document.createElement("canvas"); t.width = 64; t.height = 64; const g = t.getContext("2d")!; g.drawImage(ref.current!, 0, 0);
      send(JSON.stringify({ title: document.getElementById("title")!.textContent, pixel: Array.from(g.getImageData(2, 2, 1, 1).data).join(","), image: img.naturalWidth }));
    };
    if (img.complete) report(); else img.onload = report;
  }, []);
  useEffect(() => { if (n > 0) send("clicked:" + n); }, [n]);
  // Import-like text in a string is data, not an import (shell linking).
  return <div><h1 id="title">{title}</h1><p>{"from 'CSV' import 'react'"}</p><button id="b" onClick={() => setN(n + 1)}>Count</button>
    <canvas ref={ref} width={64} height={64} /><img id="img" src={images.dot} /></div>;
}`;

/** Every probe names a host carrying the run id, so the runner can find any
 *  attempt in Chrome's network log or its interception record. */
const probeApp = (run: string): string => `
import { useEffect } from "react";
import { useSend } from "@cowboy/dom";
const host = (name: string) => "https://" + name + "-${run}.sink.invalid";
const inner = (tag: string) => "<script>fetch('" + host(tag) + "/').catch(()=>0);try{const p=new RTCPeerConnection({iceServers:[{urls:'stun:127.0.0.1:3478'}]});p.createDataChannel('x');p.createOffer().then(o=>p.setLocalDescription(o))}catch(e){}<\\/script>";
export default function App() {
  const send = useSend();
  useEffect(() => { (async () => {
    const r: Record<string, string> = {};
    const t = async (k: string, f: () => unknown) => { try { r[k] = String(await f()); } catch (e: any) { r[k] = "threw " + (e && e.name); } };
    const link = (rel: string, href: string) => { const l = document.createElement("link"); l.rel = rel; l.href = href; document.head.append(l); return "added"; };
    await t("fetch", () => fetch(host("fetch") + "/").then((x) => x.status));
    await t("image", () => new Promise((res) => { const i = new Image(); i.onload = () => res("load"); i.onerror = () => res("error"); i.src = host("image") + "/x.png"; }));
    await t("preconnect", () => link("preconnect", host("preconnect")));
    await t("dns-prefetch", () => link("dns-prefetch", "https://dnsprefetch-${run}.example.com"));
    await t("prefetch", () => link("prefetch", host("prefetch") + "/p"));
    await t("rtc", async () => {
      const p = new RTCPeerConnection({ iceServers: [{ urls: "stun:127.0.0.1:3478" }, { urls: "turn:127.0.0.1:3478?transport=tcp", username: "u", credential: "p" }] });
      p.createDataChannel("x"); await p.setLocalDescription(await p.createOffer()); await new Promise((s) => setTimeout(s, 2500)); return p.iceGatheringState;
    });
    await t("blank", () => { const f = document.createElement("iframe"); document.body.append(f); (f.contentWindow as any).fetch(host("blank") + "/").catch(() => 0); return "executed"; });
    await t("document-write", () => { const f = document.createElement("iframe"); document.body.append(f); f.contentDocument!.write(inner("documentwrite")); f.contentDocument!.close(); return "written"; });
    await t("srcdoc", () => { const f = document.createElement("iframe"); f.srcdoc = inner("srcdoc"); document.body.append(f); return "added"; });
    await t("blob-doc", () => { const f = document.createElement("iframe"); f.src = URL.createObjectURL(new Blob([inner("blobdoc")], { type: "text/html" })); document.body.append(f); return "added"; });
    await t("data-doc", () => { const f = document.createElement("iframe"); f.src = "data:text/html," + encodeURIComponent(inner("datadoc")); document.body.append(f); return "added"; });
    await t("audio-worklet", () => new AudioContext().audioWorklet.addModule(URL.createObjectURL(new Blob(["import '" + host("audioworklet") + "/m.js';"], { type: "text/javascript" }))).then(() => "loaded"));
    await t("paint-worklet", () => (CSS as any).paintWorklet ? (CSS as any).paintWorklet.addModule(URL.createObjectURL(new Blob(["import '" + host("paintworklet") + "/m.js';"], { type: "text/javascript" }))).then(() => "loaded") : "absent");
    await t("worker", () => { new Worker(URL.createObjectURL(new Blob(["fetch('" + host("worker") + "/')"], { type: "text/javascript" }))); return "constructed"; });
    await new Promise((s) => setTimeout(s, 3000));
    send(JSON.stringify(r));
    await new Promise((s) => setTimeout(s, 500));
    location.href = host("navigate") + "/?leak=1";
  })(); }, []);
  return <p>probes</p>;
}`;

type Conformance = {
  log: string[];
  start: (kind: "boot" | "probe", run: string) => void;
  /** The `dom_app` node in the product's widget row; its passes or error. */
  product: () => Promise<{ passed?: string[]; error?: string }>;
};

const state: Conformance = {
  log: [],
  start(kind, run) {
    const container = document.createElement("div");
    container.style.width = "480px";
    document.body.append(container);
    startDomApp(
      container,
      {
        code: kind === "boot" ? BOOT_APP : probeApp(run),
        props: { title: "Hello DOM" },
        images: { dot: Uint8Array.from(atob(DOT), (c) => c.charCodeAt(0)) },
        context: { theme: "light" },
      },
      {
        onBooted: () => state.log.push("booted"),
        onHeight: () => undefined,
        onSend: (text) => state.log.push(`send:${text}`),
        onStopped: (reason) => state.log.push(`stopped:${reason}`),
      },
    );
  },
  product: () =>
    runDomAppsProductConformance().then(
      (passed) => ({ passed }),
      (error: unknown) => ({ error: String(error) }),
    ),
};

export function installDomAppsConformance(): void {
  (globalThis as typeof globalThis & { __domApps?: Conformance }).__domApps = state;
}
