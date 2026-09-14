import { assertEquals } from "jsr:@std/assert";

import { migrateThemeDefaultToSystem } from "./themeDefault.ts";

function storage(entries: readonly (readonly [string, string])[]): Storage {
  const values = new Map(entries);
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  } as Storage;
}

Deno.test("legacy seeded light theme migrates to the system default once", () => {
  const target = storage([["cowboy-theme-mode", "light"]]);

  migrateThemeDefaultToSystem(target);
  assertEquals(target.getItem("cowboy-theme-mode"), "system");
  assertEquals(target.getItem("cowboy:theme-system-default-v1"), "1");

  target.setItem("cowboy-theme-mode", "light");
  migrateThemeDefaultToSystem(target);
  assertEquals(target.getItem("cowboy-theme-mode"), "light");
});

Deno.test("theme default migration preserves dark and existing system choices", () => {
  for (const choice of ["dark", "system"] as const) {
    const target = storage([["cowboy-theme-mode", choice]]);
    migrateThemeDefaultToSystem(target);
    assertEquals(target.getItem("cowboy-theme-mode"), choice);
    assertEquals(target.getItem("cowboy:theme-system-default-v1"), "1");
  }
});
