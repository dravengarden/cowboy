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
