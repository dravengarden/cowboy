import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta } from "./protocol";
import {
  mergeSessionOverview,
  type SessionInfoPayload,
  sessionOverviewSections,
  sessionOverviewStatus,
} from "./sessionOverview.ts";

function info(overrides: Partial<SessionInfoPayload> = {}): SessionInfoPayload {
  return {
    id: "sess-1",
    provider: "codex",
    cwd: "/tmp/worktree/sess-1",
    title: "cowboy provider",
    status: "busy",
    event_count: 819,
    queue_count: 0,
    drafts_count: 3,
    ...overrides,
  };
}

function rowMap(
  payload: SessionInfoPayload,
  now?: number,
): Record<string, string> {
  return Object.fromEntries(
    sessionOverviewSections(payload, now).flatMap((section) =>
      section.rows.map((row) => [row.label, row.value])
    ),
  );
}

Deno.test("session overview fills identity workspace activity and id", () => {
  const rows = rowMap(info({
    machine_id: "hawk",
    origin: "web",
    workspace_name: "Cowboy",
    workspace_source_path: "/home/draven/columbus/projects/cowboy",
    cwd: "/home/draven/.local/state/cowboy-machine/worktrees/sess-1",
    provider_version: "0.4.2",
    context_used: 21_000,
    context_size: 353_400,
  }));
  assertEquals(rows.Title, "cowboy provider");
  assertEquals(rows.Status, "Running…");
  assertEquals(rows.Provider, "codex · 0.4.2");
  assertEquals(rows.Machine, "hawk");
  assertEquals(rows.Origin, "Cowboy");
  assertEquals(rows.Project, "Cowboy");
  assertEquals(
    rows.Checkout,
    "/home/draven/columbus/projects/cowboy",
  );
  assertEquals(
    rows.Worktree,
    "/home/draven/.local/state/cowboy-machine/worktrees/sess-1",
  );
  assertEquals(rows.Directory, undefined);
  assertEquals(rows.Context, "21,000 / 353,400 · 6%");
  assertEquals(rows.Events, "819");
  assertEquals(rows.Queued, "0");
  assertEquals(rows.Drafts, "3");
  assertEquals(rows["Session ID"], "sess-1");
  assertEquals(rows["Next draft"], undefined);
});

Deno.test("session overview keeps a single directory when checkout is the cwd", () => {
  const rows = rowMap(info({
    workspace_source_path: "/tmp/worktree/sess-1",
  }));
  assertEquals(rows.Directory, "/tmp/worktree/sess-1");
  assertEquals(rows.Checkout, undefined);
  assertEquals(rows.Worktree, undefined);
  assertEquals(rows.Project, "Not recorded");
  assertEquals(rows.Machine, "local");
  assertEquals(rows.Origin, "External");
  assertEquals(rows.Context, "Not reported");
});

Deno.test("session overview status reports lifecycle and manual queue pause", () => {
  assertEquals(sessionOverviewStatus(info()), "Running…");
  assertEquals(
    sessionOverviewStatus(info({ paused: true })),
    "Running… · Queue paused",
  );
  assertEquals(
    sessionOverviewStatus(info({
      status: "running",
      paused: true,
    })),
    "Live · Queue paused",
  );
});

Deno.test("system sessions get an explicit kind row", () => {
  const rows = rowMap(info({ system: true, status: "running" }));
  assertEquals(rows.Kind, "System");
  assertEquals(rows.Status, "Live");
});

Deno.test("next scheduled draft uses the compact Chinese fire label", () => {
  const now = new Date(2026, 7, 20, 19, 15).getTime();
  const fire = new Date(2026, 7, 20, 20, 0).getTime();
  const rows = rowMap(info({ next_schedule_ms: fire }), now);
  assertEquals(rows["Next draft"], "今天 20:00 · 45 分钟后");
});

Deno.test("merge keeps HTTP counts and takes live status context", () => {
  const live: SessionMeta = {
    id: "sess-1",
    provider: "codex",
    cwd: "/tmp/worktree/sess-1",
    title: "renamed",
    status: "running",
    paused: true,
    context_used: 80,
    context_size: 100,
  };
  const merged = mergeSessionOverview(
    info({ event_count: 819, queue_count: 2, drafts_count: 3 }),
    live,
  );
  assertEquals(merged.title, "renamed");
  assertEquals(merged.status, "running");
  assertEquals(merged.paused, true);
  assertEquals(merged.event_count, 819);
  assertEquals(merged.queue_count, 2);
  assertEquals(merged.drafts_count, 3);
  assertEquals(merged.context_used, 80);
});

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("session info surfaces render the shared overview sections", () => {
  assertEquals(appSource.includes("sessionOverviewSections("), true);
  assertEquals(appSource.includes("mergeSessionOverview("), true);
  assertEquals(
    appSource.includes('<InfoRow k="Directory" v={info.cwd} />'),
    false,
  );
});
