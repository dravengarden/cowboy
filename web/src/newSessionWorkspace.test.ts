import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace.ts";
import { continuationOf, resolveActiveSession } from "./sessionSelection.ts";
import type { SessionMeta } from "./protocol.ts";
import { declarationSource, readAppSource, readComposerSource, readTranscriptSource } from "./appSource.test.ts";

const appSource = await readAppSource();
const transcriptSource = await readTranscriptSource();
const composerSource = await readComposerSource();

test("a session with its first message on the way drops the setup form", () => {
  assertEquals(transcriptSource.includes("{!committed && <SessionStartSettings"), true);
  assertEquals(transcriptSource.includes("{!committed && (\n        <ButtonBase"), true);
});

test("mobile new session actions stay in the non-overlay sheet footer", () => {
  const dialog = declarationSource(appSource, "CreateDialog");
  assertEquals(dialog.includes("<MobileDecisionActions"), true);
  assertEquals(dialog.includes("shelf"), true);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-footer-actions"), true);
  assertEquals(dialog.includes("data-new-session-sticky-actions"), false);
  assertEquals(dialog.includes('position: "sticky"'), false);
  assertEquals(dialog.includes("footer={"), true);
  assertEquals(dialog.includes("SHEET_THUMB_CLEARANCE"), false);
  assertEquals(dialog.includes("title={dialogTitle}"), true);
});

test("new session navigation precedes Machine preparation completion", () => {
  const created = appSource.indexOf("onCreated={(session): void => {");
  const active = appSource.indexOf("setActiveId(session.id);", created);
  const settle = appSource.indexOf(
    "settleMobileDrawerRef.current(false, 0);",
    created,
  );
  assertEquals(created >= 0 && active > created && settle > active, true);
  assertEquals(
    /<ConversationEmptyState\s+sessionId=\{sessionId\}\s+kind="preparing"/u.test(transcriptSource),
    true,
  );
  assertEquals(transcriptSource.includes("Preparing session"), true);
  assertEquals(
    transcriptSource.includes("Creating an isolated workspace"),
    true,
  );
  assertEquals(
    /placeholder=\{preparing\s*\?\s*"You can start typing while this session prepares…"/
      .test(composerSource),
    true,
  );
  // Startup is presented on session-level surfaces (StatusDot and the
  // transcript empty state) — never by replacing the composer's primary action
  // with a spinner, never by unmounting toolbar actions (the row would reflow
  // on the ready edge), and not by a second progress mark on the composer card.
  assertEquals(
    composerSource.includes('aria-label="preparing session"'),
    false,
  );
  assertEquals(composerSource.includes("SessionPreparingLine"), false);
  assertEquals(
    composerSource.includes("{!preparing && !desktop && compactAction"),
    false,
  );
  assertEquals(composerSource.includes("{!preparing && !compact &&"), false);
  assertEquals(composerSource.includes("{!preparing && clearAction"), false);
  // Writing during startup must reach the daemon queue: the session is not
  // dispatchable yet, so submitPrompt queues and the daemon drains it on the
  // Running edge. A local guard here would make the placeholder a lie.
  assertEquals(composerSource.includes("if (preparing) return false;"), false);
  assertEquals(composerSource.includes("if (preparing) return;"), false);
  const opener = appSource.slice(
    appSource.indexOf("const openNewSession = (): void => {"),
    appSource.indexOf("const [pendingCreatedSession"),
  );
  assertEquals(opener.includes("if (mobile) claimKeyboard();"), true);
  assertEquals(
    appSource.includes("const openNewSession = (): void => {"),
    true,
  );
  assertEquals(
    /titleRef\.current\?\.focus\(\{ preventScroll: true \}\);\s*titleRef\.current\?\.select\(\);\s*\}, 120\);/u
      .test(appSource),
    true,
  );
  assertEquals(appSource.includes("initial_prompt: selectedWorkItem"), true);
  assertEquals(appSource.includes("/prompt`, {"), false);
  assertEquals(
    appSource.includes("snapshot.configOptions.get(sessionId) ?? []"),
    false,
  );
});

test("new session stays selected before the sessions broadcast arrives", () => {
  const existing: SessionMeta = {
    id: "existing",
    provider: "codex",
    cwd: "/workspace/existing",
    title: "Existing",
    status: "running",
  };
  const pending: SessionMeta = {
    ...existing,
    id: "created",
    cwd: "/workspace/created",
    title: "New session",
    status: "starting",
  };

  assertEquals(resolveActiveSession([existing], pending.id, pending), pending);
  assertEquals(
    resolveActiveSession([pending, existing], pending.id, pending),
    pending,
  );
});

test("a replaced session reopens its continuation, not the first session", () => {
  const base: SessionMeta = {
    id: "first",
    provider: "codex",
    cwd: "/workspace",
    title: "First",
    status: "running",
  };
  const second: SessionMeta = { ...base, id: "second", continued_from: "source" };
  const third: SessionMeta = { ...base, id: "third", continued_from: "second" };

  // The source was trashed by Replace; only its continuation is listed.
  assertEquals(resolveActiveSession([base, second], "source", null), second);
  // A continuation that was itself continued resolves to the newest hop.
  assertEquals(resolveActiveSession([base, second, third], "source", null), third);
  assertEquals(continuationOf([base], "source"), undefined);
  assertEquals(resolveActiveSession([base], "source", null), base);
  // A middle hop that was replaced too is walked through, not opened.
  assertEquals(resolveActiveSession([base, third], "source", null, [second]), third);
  assertEquals(continuationOf([base], "source", [second]), undefined);
  // A cycle never loops.
  const loopA: SessionMeta = { ...base, id: "a", continued_from: "b" };
  const loopB: SessionMeta = { ...base, id: "b", continued_from: "a" };
  assertEquals(continuationOf([loopA, loopB], "a")?.id, "b");
  // A source kept open and handed off again follows its latest handoff,
  // which wire order lists after the earlier one.
  const earlier: SessionMeta = { ...base, id: "earlier", continued_from: "source" };
  const later: SessionMeta = { ...base, id: "later", continued_from: "source" };
  assertEquals(continuationOf([earlier, later], "source")?.id, "later");
  assertEquals(continuationOf([earlier], "source", [later])?.id, "earlier");
});

test("new sessions respect Machine workspace ordering", () => {
  const choices = [
    {
      value: "cowboy",
      label: "cowboy",
      help: "/home/draven/columbus/projects/cowboy",
    },
    { value: "columbus", label: "columbus", help: "/home/draven/columbus" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "cowboy");
});

test("new session workspace falls back to the first available choice", () => {
  const choices = [
    { value: "remote-root", label: "Remote", help: "/srv/work" },
    { value: "other", label: "Other", help: "/srv/other" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "remote-root");
});
