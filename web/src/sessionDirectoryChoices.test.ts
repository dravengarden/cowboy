import { assertEquals } from "jsr:@std/assert";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices.ts";
import {
  effectiveSessionFolder,
  sessionFolderMutators,
} from "./sessionFolders.ts";
import type { SessionMeta } from "./protocol.ts";
Deno.test("session directory choices expose empty Global and exact nested folder identities", () => {
  const folders = {
    folders: [
      {
        id: "parent",
        name: "Work",
        parent: null,
        position: 0,
        project: "cowboy",
      },
      {
        id: "child",
        name: "Cowboy",
        parent: "parent",
        position: 0,
        project: null,
      },
    ],
    placement: {},
  };
  const choices = sessionDirectoryChoices(folders);
  assertEquals(choices[0].value, "");
  assertEquals(choices[0].label, "Global");
  assertEquals(choices[2].value, "child");
  assertEquals(choices[2].hierarchyPath, ["Work", "Cowboy"]);
  const session = { id: "new", workspace_name: "cowboy" } as SessionMeta;
  assertEquals(effectiveSessionFolder(session, folders), "parent");
  assertEquals(
    effectiveSessionFolder(
      session,
      sessionFolderMutators.place(folders, {
        session_ids: [session.id],
        folder: null,
      }),
    ),
    null,
  );
  assertEquals(
    effectiveSessionFolder(
      session,
      sessionFolderMutators.place(folders, {
        session_ids: [session.id],
        folder: "child",
      }),
    ),
    "child",
  );
});
