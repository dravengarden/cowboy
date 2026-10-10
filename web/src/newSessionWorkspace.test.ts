import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace.ts";
import { resolveActiveSession } from "./sessionSelection.ts";
import type { SessionMeta } from "./protocol.ts";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const transcriptSource = await readFile(
  new URL("./Transcript.tsx", import.meta.url), "utf8",
);
const composerSource = await readFile(
  new URL("./Composer.tsx", import.meta.url), "utf8",
);

test("mobile new session actions stay in the non-overlay sheet footer", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function CreateDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionActions"), true);
  assertEquals(dialog.includes("shelf"), true);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-footer-actions"), true);
  assertEquals(dialog.includes("data-new-session-sticky-actions"), false);
  assertEquals(dialog.includes('position: "sticky"'), false);
  assertEquals(dialog.includes("footer={"), true);
  assertEquals(dialog.includes("SHEET_THUMB_CLEARANCE"), false);
  assertEquals(dialog.includes('title="Create"'), true);
});

test("new session navigation precedes Machine preparation completion", () => {
  const created = appSource.indexOf("onCreated={(session, folder): void => {");
  const active = appSource.indexOf("setActiveId(session.id);", created);
  const settle = appSource.indexOf(
    "settleMobileDrawerRef.current(false, 0);",
    created,
  );
  assertEquals(created >= 0 && active > created && settle > active, true);
  assertEquals(
    /<ConversationEmptyState\s+kind="preparing"/u.test(transcriptSource),
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
