import { assertEquals } from "jsr:@std/assert";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace.ts";

const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);

Deno.test("new session navigation precedes Machine preparation completion", () => {
  const created = appSource.indexOf("onCreated={(id): void => {");
  const active = appSource.indexOf("setActiveId(id);", created);
  const settle = appSource.indexOf("settleMobileDrawerRef.current(false, 0);", created);
  assertEquals(created >= 0 && active > created && settle > active, true);
  assertEquals(
    transcriptSource.includes('<ConversationEmptyState kind="preparing"'),
    true,
  );
  assertEquals(transcriptSource.includes("Preparing session"), true);
  assertEquals(
    transcriptSource.includes("Creating an isolated workspace"),
    true,
  );
  assertEquals(
    composerSource.includes('placeholder={preparing\n              ? "You can start typing while this session prepares…"'),
    true,
  );
  assertEquals(composerSource.includes('aria-label="preparing session"'), true);
  assertEquals(composerSource.includes("if (preparing) return false;"), true);
  assertEquals(appSource.includes("initial_prompt: selectedWorkItem"), true);
  assertEquals(appSource.includes("/prompt`, {"), false);
});

Deno.test("new sessions prefer Columbus regardless of workspace ordering", () => {
  const choices = [
    { value: "cowboy", label: "cowboy", help: "/home/draven/columbus/projects/cowboy" },
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
