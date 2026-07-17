import { assert, assertEquals } from "jsr:@std/assert";
import {
  composerSessionSlice,
  sameComposerSessionSlice,
  sameComposerSheetSession,
} from "./composerSessionSlice";
import type { SessionMeta } from "./protocol";

const active: SessionMeta = {
  id: "active",
  provider: "codex",
  cwd: "/active",
  title: "Active",
  status: "busy",
  paused: false,
  context_used: 10,
  context_size: 100,
};
const other: SessionMeta = {
  id: "other",
  provider: "codex",
  cwd: "/other",
  title: "Other",
  status: "running",
};

Deno.test("composer session slice ignores invisible session-list churn", () => {
  const before = composerSessionSlice([active, other], active.id);
  const after = composerSessionSlice([
    {
      ...active,
      status: "running",
      usage: { used: 10, size: 100, raw: null, observed_at_ms: 2 },
    },
    { ...other, status: "busy", context_used: 80, context_size: 100 },
  ], active.id);

  assert(sameComposerSessionSlice(before, after));
  assertEquals(after.destinations, [{
    id: "other",
    title: "Other",
    cwd: "/other",
  }]);
});

Deno.test("composer session slice reacts to every visible composer field", () => {
  const initial = composerSessionSlice([active, other], active.id);
  const changes: SessionMeta[][] = [
    [{ ...active, provider: "claude-code" }, other],
    [{ ...active, paused: true }, other],
    [{ ...active, awaiting_user: true }, other],
    [{ ...active, done: true }, other],
    [{ ...active, judging: true }, other],
    [{ ...active, context_used: 11 }, other],
    [{ ...active, context_size: 101 }, other],
    [active, { ...other, title: "Renamed" }],
    [active, { ...other, cwd: "/moved" }],
    [active],
  ];

  for (const sessions of changes) {
    assert(
      !sameComposerSessionSlice(
        initial,
        composerSessionSlice(sessions, active.id),
      ),
    );
  }
});

Deno.test("composer sheet session ignores metadata it does not render", () => {
  assert(sameComposerSheetSession(active, {
    ...active,
    usage: { used: 10, size: 100, raw: null, observed_at_ms: 2 },
    context_used: 40,
    context_size: 200,
    judging: true,
    next_schedule_ms: 123,
  }));
  assert(!sameComposerSheetSession(active, { ...active, title: "Renamed" }));
  assert(!sameComposerSheetSession(active, { ...active, paused: true }));
  assert(!sameComposerSheetSession(active, { ...active, status: "running" }));
});
