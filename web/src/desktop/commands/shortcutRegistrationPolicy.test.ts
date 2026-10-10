import { readdir, readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals, assertStringIncludes } from "@std/assert";
import {
  type RegisteredShortcut,
  shortcutRegistrationConflict,
} from "./shortcutRegistrationPolicy.ts";

function stringList(block: string, field: string): string[] | undefined {
  const match = new RegExp("\\b" + field + ": \\[([^\\]]*)\\]").exec(block);
  return match?.[1]
    ? [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1] as string)
    : undefined;
}

/** Every literal dotted-id command declaring a direct shortcut in web/src. */
async function declaredDirectShortcuts(): Promise<RegisteredShortcut[]> {
  const root = new URL("../../", import.meta.url);
  const declared: RegisteredShortcut[] = [];
  for (const entry of await readdir(root, { recursive: true })) {
    if (!/\.tsx?$/.test(entry) || /\.test\.tsx?$/.test(entry)) continue;
    const source = await readFile(new URL(entry, root), "utf8");
    // A declaration runs from its command id to its handler.
    for (const part of source.split(/\bid: (?="[a-z]+\.[A-Za-z.]+",)/).slice(1)) {
      const id = /^"([^"]+)"/.exec(part)?.[1];
      const end = part.search(/\brun: /);
      const block = end < 0 ? part.slice(0, 1200) : part.slice(0, end);
      const shortcut = /\bshortcut: "([^"]+)"/.exec(block)?.[1];
      if (!id || !shortcut) continue;
      const surface = /\bsurface: "(session|document)"/.exec(block)?.[1] as
        | RegisteredShortcut["surface"]
        | undefined;
      const contexts = stringList(block, "contexts");
      const regions = stringList(block, "regions");
      declared.push({
        id,
        shortcut,
        ...(contexts ? { contexts } : {}),
        ...(regions ? { regions } : {}),
        ...(surface ? { surface } : {}),
      });
    }
  }
  return declared;
}

test("declared direct shortcuts never overlap once mounted together", async () => {
  // Registration throws during render, so a clash between two commands that
  // only meet on one surface (a Top Bar letter and the Page view's P) takes
  // the whole Desktop down the first time both mount.
  const declared = await declaredDirectShortcuts();
  for (
    const id of [
      "topbar.providerUpdate",
      "conversation.toggleQuestionDirectory",
      "conversation.toggleFollow",
    ]
  ) {
    assert(declared.some((command) => command.id === id), id + " not scanned");
  }
  const conflicts = declared.flatMap((command) => {
    const conflict = shortcutRegistrationConflict(command, declared);
    return conflict ? [command.id + ": " + conflict] : [];
  });
  assertEquals(conflicts, []);
});

test("global bare product letters are forbidden", () => {
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

test("overlapping direct shortcuts cannot shadow each other", () => {
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

test("a prefix continuation has only one command meaning", () => {
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

test("scoped editors may share one leader meaning in disjoint regions", () => {
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

test("Session and Draft surfaces may reuse a leader key", () => {
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
