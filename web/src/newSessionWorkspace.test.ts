import { assertEquals } from "jsr:@std/assert";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace.ts";
import { resolveActiveSession } from "./sessionSelection.ts";
import type { SessionMeta } from "./protocol.ts";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("mobile new session actions stay at the sticky form end", () => {
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes("<MobileDecisionDock"), false);
  assertEquals(dialog.includes("footerOverlay"), false);
  assertEquals(dialog.includes("data-new-session-sticky-actions"), true);
  assertEquals(dialog.includes('position: "sticky"'), true);
  assertEquals(dialog.includes("SHEET_THUMB_CLEARANCE"), false);
  assertEquals(dialog.includes('title="New session"'), true);
});

Deno.test("new session navigation precedes Machine preparation completion", () => {
  const created = appSource.indexOf("onCreated={(session): void => {");
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
  assertEquals(composerSource.includes('aria-label="preparing session"'), true);
  assertEquals(composerSource.includes("if (preparing) return false;"), true);
  assertEquals(appSource.includes("if (mobile) claimKeyboard();"), true);
  assertEquals(
    appSource.includes("const openNewSession = (): void => {"),
    true,
  );
  assertEquals(
    appSource.includes(
      "titleRef.current?.focus({ preventScroll: true });\n            titleRef.current?.select();\n        }, 120);",
    ),
    true,
  );
  assertEquals(appSource.includes("initial_prompt: selectedWorkItem"), true);
  assertEquals(appSource.includes("/prompt`, {"), false);
  assertEquals(
    appSource.includes("snapshot.configOptions.get(sessionId) ?? []"),
    false,
  );
});

Deno.test("new session stays selected before the sessions broadcast arrives", () => {
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

Deno.test("new sessions prefer Columbus regardless of workspace ordering", () => {
  const choices = [
    {
      value: "cowboy",
      label: "cowboy",
      help: "/home/draven/columbus/projects/cowboy",
    },
    { value: "columbus", label: "columbus", help: "/home/draven/columbus" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "columbus");
});

Deno.test("new session workspace falls back to the first available choice", () => {
  const choices = [
    { value: "remote-root", label: "Remote", help: "/srv/work" },
    { value: "other", label: "Other", help: "/srv/other" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "remote-root");
});

Deno.test("new session provider marks are centred in their leading column", () => {
  assertEquals(
    appSource.includes(
      'sx={{ alignItems: "center", py: 1, whiteSpace: "normal" }}',
    ),
    true,
  );
  assertEquals(
    appSource.includes(
      'sx={{ width: 36, minWidth: 36, justifyContent: "center" }}',
    ),
    true,
  );
});
