import { assertEquals, assertStringIncludes } from "jsr:@std/assert";
import { shortcutRegistrationConflict } from "./shortcutRegistrationPolicy.ts";

Deno.test("global bare product letters are forbidden", () => {
  assertStringIncludes(
    shortcutRegistrationConflict({ id: "global", shortcut: "S" }, []) ?? "",
    "global bare product letter",
  );
  assertEquals(
    shortcutRegistrationConflict({
      id: "scoped",
      shortcut: "S",
      regions: ["sessions"],
    }, []),
    null,
  );
});

Deno.test("overlapping direct shortcuts cannot shadow each other", () => {
  const global = { id: "global", shortcut: "Mod+." };
  assertStringIncludes(
    shortcutRegistrationConflict(
      { id: "scoped", shortcut: "Mod+.", regions: ["prompt"] },
      [global],
    ) ?? "",
    "overlaps global",
  );
  assertEquals(
    shortcutRegistrationConflict(
      { id: "conversation", shortcut: "R", regions: ["conversation"] },
      [{ id: "topbar", shortcut: "R", regions: ["topbar"] }],
    ),
    null,
  );
});

Deno.test("a prefix continuation has only one command meaning", () => {
  const existing = { id: "sessions", sequence: ["Mod+K", "S"] };
  assertStringIncludes(
    shortcutRegistrationConflict(
      { id: "other", sequence: ["Mod+K", "s"] },
      [existing],
    ) ?? "",
    "already belongs to sessions",
  );
  assertEquals(
    shortcutRegistrationConflict(
      { id: "prompt", sequence: ["Mod+K", "P"] },
      [existing],
    ),
    null,
  );
});

Deno.test("scoped editors may share one leader meaning in disjoint regions", () => {
  assertEquals(
    shortcutRegistrationConflict(
      { id: "queued.slash", sequence: ["Mod+K", "/"], regions: ["prompt.queued"] },
      [{ id: "composer.slash", sequence: ["Mod+K", "/"], regions: ["prompt.composer"] }],
    ),
    null,
  );
  assertStringIncludes(
    shortcutRegistrationConflict(
      { id: "global.slash", sequence: ["Mod+K", "/"] },
      [{ id: "composer.slash", sequence: ["Mod+K", "/"], regions: ["prompt.composer"] }],
    ) ?? "",
    "already belongs to composer.slash",
  );
});

Deno.test("Session and Draft surfaces may reuse a leader key", () => {
  const schedule = {
    id: "composer.schedule",
    sequence: ["Mod+K", "H"],
    contexts: ["prompt"],
    regions: ["prompt.composer"],
    surface: "session" as const,
  };
  const history = {
    id: "document.history",
    sequence: ["Mod+K", "H"],
    surface: "document" as const,
  };
  assertEquals(shortcutRegistrationConflict(history, [schedule]), null);
  assertStringIncludes(
    shortcutRegistrationConflict({ ...history, surface: "session" }, [schedule]) ??
      "",
    "already belongs to composer.schedule",
  );
});
