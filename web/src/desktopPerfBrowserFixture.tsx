/** Desktop performance fixture: the real Desktop App, store and Transcript
 * under a production-sized workload (hundreds of sessions, several busy,
 * long tool-heavy transcripts, a live token stream and status churn).
 * No account, server or agent: fetch and the WebSocket are in-page fakes.
 *
 * `tools/desktop-perf-profile.ts` drives it over CDP and records CPU profiles
 * and frame/long-task measurements per scenario. An optional `/perf-data.json`
 * (`{ bootstraps: { [sessionId]: messages } }`) replaces the synthetic
 * transcripts with exported ones for local investigation only.
 */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { App } from "./App";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { DesktopWorkspaceProvider } from "./desktop/DesktopWorkspaceController";
import { DesktopCommandProvider } from "./desktop/commands/DesktopCommandProvider";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { managementEntryFixture } from "./providerManagement.fixture";
import {
  loadProviderCatalog,
  resetProviderCatalog,
} from "./providerCatalogRegistry";
import type { Envelope, SessionMeta, Status } from "./protocol";

const SESSIONS = 250;
const BUSY = 8;

const delay = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));
async function until(
  predicate: () => boolean,
  label: string,
  ms = 10_000,
): Promise<void> {
  const began = performance.now();
  while (!predicate()) {
    if (performance.now() - began > ms) throw new Error(`Timed out: ${label}`);
    await delay(10);
  }
}

const CODE = Array.from(
  { length: 40 },
  (_, i) =>
    `    let value_${i} = compute(${i}, &state).await?; // keep the borrow short`,
).join("\n");
const PROSE =
  "The transcript keeps every tool call, diff and message so a reviewer can follow the " +
  "agent's reasoning. **Bold**, `inline code`, [a link](https://example.invalid) and a list:\n\n" +
  "- first item\n- second item with `code`\n- third item\n\n";

/** A tool-heavy turn shaped like an exported production transcript. */
function syntheticTimeline(
  sessionId: string,
  events: number,
  start = 1,
): Envelope[] {
  const out: Envelope[] = [];
  let seq = start;
  for (let i = 0; out.length < events; i++) {
    out.push({
      seq: seq++,
      kind: "update",
      session_id: sessionId,
      update: {
        sessionUpdate: "agent_thought_chunk",
        content: { type: "text", text: `Thinking about step ${i}. ${PROSE}` },
      },
    } as unknown as Envelope);
    out.push({
      seq: seq++,
      kind: "update",
      session_id: sessionId,
      update: {
        sessionUpdate: "tool_call",
        toolCallId: `${sessionId}-tool-${i}`,
        title: i % 3 === 0 ? "Editing files" : "Run command",
        kind: i % 3 === 0 ? "edit" : "execute",
        status: "completed",
        content: i % 3 === 0
          ? [{
            type: "diff",
            path: `/fixture/src/module_${i}.rs`,
            oldText: CODE,
            newText: CODE.replaceAll("short", "tight"),
          }]
          : [{
            type: "content",
            content: {
              type: "text",
              text: `$ cargo test -p fixture_${i}\n${CODE}\n`,
            },
          }],
      },
    } as unknown as Envelope);
    if (i % 4 === 3) {
      out.push({
        seq: seq++,
        kind: "update",
        session_id: sessionId,
        update: {
          sessionUpdate: "agent_message_chunk",
          content: {
            type: "text",
            text: `## Result ${i}\n\n${PROSE}\`\`\`rust\n${CODE}\n\`\`\`\n`,
          },
        },
      } as unknown as Envelope);
    }
  }
  return out;
}

interface PerfApi {
  ready: Promise<void>;
  sessions: SessionMeta[];
  /** Live token stream into the opened session plus status churn elsewhere. */
  stream(
    ms: number,
    opts?: { chunkMs?: number; churnMs?: number },
  ): Promise<void>;
  /** Only `sessions` broadcasts: busy rows change activity every `everyMs`. */
  churn(ms: number, everyMs: number): Promise<number>;
  /** Activate `count` different session rows one after another. */
  switchSessions(count: number): Promise<number[]>;
  measure<T>(
    run: () => Promise<T>,
  ): Promise<
    {
      result: T;
      frames: number;
      longTasks: number;
      longTaskMs: number;
      maxFrameMs: number;
      p95FrameMs: number;
      durationMs: number;
    }
  >;
}

export async function runDesktopPerfFixture(): Promise<string[]> {
  let realBootstraps: Record<string, unknown[]> = {};
  try {
    const response = await fetch("/perf-data.json");
    if (response.ok) {
      realBootstraps =
        ((await response.json()) as { bootstraps: Record<string, unknown[]> })
          .bootstraps;
    }
  } catch {
    // Synthetic data only.
  }
  const realIds = Object.keys(realBootstraps);
  const now = Date.now();
  const sessions: SessionMeta[] = Array.from({ length: SESSIONS }, (_, i) => {
    const id = realIds[i] ?? `perf-${String(i).padStart(3, "0")}`;
    const status: Status = i < BUSY
      ? "busy"
      : i < BUSY + 15
      ? "running"
      : "exited";
    return {
      id,
      title: `Perf session ${i} ${i < BUSY ? "busy" : ""}`.trim(),
      provider: "example",
      cwd: `/fixture/project-${i % 12}`,
      machine_id: i % 2 ? "ovh" : "hawk",
      status,
      origin: "web",
      workspace_id: `project-${i % 12}`,
      workspace_name: `project-${i % 12}`,
      created_at_ms: now - (SESSIONS - i) * 60_000,
      updated_at_ms: now - (SESSIONS - i) * 1000,
      context_used: 40_000 + i,
      context_size: 200_000,
    } as SessionMeta;
  });
  const timelines = new Map<string, Envelope[]>();
  const timelineOf = (id: string): Envelope[] => {
    let timeline = timelines.get(id);
    if (!timeline) {
      timeline = syntheticTimeline(id, 160);
      timelines.set(id, timeline);
    }
    return timeline;
  };

  class FixtureSocket {
    static current: FixtureSocket;
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 1;
    protocol = "cowboy-sync-v1";
    bufferedAmount = 0;
    onopen: (() => void) | null = null;
    onmessage: ((event: { data: string }) => void) | null = null;
    onclose = null;
    onerror = null;
    constructor() {
      FixtureSocket.current = this;
      setTimeout(() => {
        this.onopen?.();
        this.publish({ type: "sessions", sessions });
        this.publish({ type: "bootstrap_complete" });
      }, 20);
    }
    publish(message: unknown) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
    send() {}
    close() {
      this.readyState = 3;
    }
  }
  globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;
  bindProductSyncPrincipal("perf-fixture");
  localStorage.setItem("cowboy:active", sessions[0]!.id);
  const entry = managementEntryFixture("example");
  entry.publisher = entry.manifest.publisher;
  entry.manifest.authentication.required = false;
  entry.manifest.authentication.methods = [];
  const originalFetch = globalThis.fetch;
  const fetchCounts = new Map<string, number>();
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input instanceof Request ? input.url : input);
    const url = new URL(path, location.href);
    if (!url.pathname.startsWith("/api/")) return originalFetch(input, init);
    fetchCounts.set(url.pathname, (fetchCounts.get(url.pathname) ?? 0) + 1);
    if (url.pathname === "/api/sync/dataset") {
      return Response.json({
        schema: "dravengarden.cowboy.product-sync-dataset/v1",
        dataset_id: `dataset-${"a".repeat(64)}`,
        user_id: "perf-fixture",
        database_version: 2,
        outbox_contract: "atomic-delta-v1",
      });
    }
    if (url.pathname === "/api/plugins") {
      return Response.json({
        providers: [entry],
        authentications: [],
        authentication_executors: [],
      });
    }
    const bootstrap = /^\/api\/sessions\/([^/]+)\/bootstrap$/.exec(
      url.pathname,
    );
    if (bootstrap) {
      const id = decodeURIComponent(bootstrap[1]!);
      await delay(30);
      const real = realBootstraps[id];
      if (real) return Response.json({ messages: real });
      return Response.json({
        messages: [{
          type: "snapshot",
          session_id: id,
          events: timelineOf(id),
          reached_start: true,
        }],
      });
    }
    if (url.pathname === "/api/usage") {
      return Response.json({ providers: [], observed_at_ms: Date.now() });
    }
    return new Response("Isolated fixture", { status: 404 });
  }) as typeof fetch;
  resetProviderCatalog();
  await loadProviderCatalog();

  const container = document.createElement("div");
  container.style.cssText = "height:100vh;width:100vw;overflow:hidden";
  document.body.append(container);
  const root = createRoot(container);
  flushSync(() =>
    root.render(
      <BrowserProductTheme>
        <CssBaseline />
        <SurfaceProvider>
          <DesktopWorkspaceProvider>
            <DesktopCommandProvider>
              <App
                themeMode={localStorage.getItem("cowboy-theme-mode") === "dark"
                  ? "dark"
                  : "light"}
                onSetThemeMode={() => {}}
                surface="desktop"
              />
            </DesktopCommandProvider>
          </DesktopWorkspaceProvider>
        </SurfaceProvider>
      </BrowserProductTheme>,
    )
  );
  const rows = () => [
    ...container.querySelectorAll<HTMLElement>("[data-desktop-session-row]"),
  ];
  const transcriptRows = () =>
    container.querySelectorAll("[data-desktop-transcript-scroller] [data-key]")
      .length;
  const ready = (async () => {
    await until(() => rows().length > 0, "session rows");
    rows()[0]!.click();
    await until(() => transcriptRows() > 0, "transcript rows");
  })();
  await ready;

  let longTasks: PerformanceEntry[] = [];
  new PerformanceObserver((list) => {
    longTasks = longTasks.concat(list.getEntries());
  }).observe({ type: "longtask" });

  const measure: PerfApi["measure"] = async (run) => {
    longTasks = [];
    const frames: number[] = [];
    let last = performance.now();
    let running = true;
    const tick = (time: number) => {
      frames.push(time - last);
      last = time;
      if (running) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    const began = performance.now();
    const result = await run();
    const durationMs = performance.now() - began;
    running = false;
    await delay(50);
    const sorted = [...frames].sort((a, b) => a - b);
    return {
      result,
      frames: frames.length,
      durationMs,
      longTasks: longTasks.length,
      longTaskMs: longTasks.reduce((sum, task) => sum + task.duration, 0),
      maxFrameMs: sorted.at(-1) ?? 0,
      p95FrameMs: sorted[Math.floor(sorted.length * 0.95)] ?? 0,
    };
  };

  let liveSeq = 10_000_000;
  const stream: PerfApi["stream"] = async (ms, opts = {}) => {
    const chunkMs = opts.chunkMs ?? 30;
    const churnMs = opts.churnMs ?? 1000;
    const opened = localStorage.getItem("cowboy:active") ?? sessions[0]!.id;
    const socket = FixtureSocket.current;
    const words = PROSE.split(/(?<= )/);
    const end = performance.now() + ms;
    let nextChurn = performance.now() + churnMs;
    let word = 0;
    let tool = 0;
    while (performance.now() < end) {
      for (
        const sessionId of [
          opened,
          ...sessions.slice(1, BUSY).map((s) => s.id),
        ]
      ) {
        socket.publish({
          type: "event",
          envelope: {
            seq: ++liveSeq,
            kind: "update",
            session_id: sessionId,
            update: {
              sessionUpdate: "agent_message_chunk",
              content: { type: "text", text: words[word % words.length] },
            },
          },
        });
      }
      word++;
      if (word % 40 === 0) {
        tool++;
        socket.publish({
          type: "event",
          envelope: {
            seq: ++liveSeq,
            kind: "update",
            session_id: opened,
            update: {
              sessionUpdate: "tool_call",
              toolCallId: `live-${tool}`,
              title: "Run command",
              kind: "execute",
              status: "completed",
              content: [{
                type: "content",
                content: { type: "text", text: CODE },
              }],
            },
          },
        });
      }
      if (performance.now() >= nextChurn) {
        nextChurn += churnMs;
        const stamp = Date.now();
        socket.publish({
          type: "sessions",
          sessions: sessions.map((s, i) =>
            i < BUSY
              ? {
                ...s,
                updated_at_ms: stamp,
                context_used: (s.context_used ?? 0) + word,
              }
              : s
          ),
        });
      }
      await delay(chunkMs);
    }
  };

  const switchSessions: PerfApi["switchSessions"] = async (count) => {
    const timings: number[] = [];
    for (let i = 1; i <= count; i++) {
      const target = rows()[i % rows().length]!;
      const began = performance.now();
      target.click();
      await new Promise((resolve) =>
        requestAnimationFrame(() => setTimeout(resolve, 0))
      );
      await until(() => transcriptRows() > 0, "switched transcript");
      timings.push(performance.now() - began);
      await delay(200);
    }
    return timings;
  };

  const churn: PerfApi["churn"] = async (ms, everyMs) => {
    const end = performance.now() + ms;
    let broadcasts = 0;
    while (performance.now() < end) {
      const stamp = Date.now();
      FixtureSocket.current.publish({
        type: "sessions",
        sessions: sessions.map((s, i) =>
          i < BUSY
            ? {
              ...s,
              updated_at_ms: stamp,
              context_used: (s.context_used ?? 0) + broadcasts,
            }
            : s
        ),
      });
      broadcasts++;
      await delay(everyMs);
    }
    return broadcasts;
  };

  const api: PerfApi = {
    ready,
    sessions,
    stream,
    churn,
    switchSessions,
    measure,
  };
  Object.assign(globalThis, {
    cowboyPerf: api,
    cowboyPerfFetches: fetchCounts,
  });
  return ["Desktop perf fixture mounted"];
}
