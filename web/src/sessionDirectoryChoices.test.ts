import { assertEquals } from "jsr:@std/assert";
import { sessionDirectoryChoices } from "./sessionDirectoryChoices.ts";
import {
  effectiveSessionFolder,
  sessionFolderMutators,
} from "./sessionFolders.ts";
import type { SessionMeta } from "./protocol.ts";
Deno.test("session directory choices contain only folders; empty placement remains Global", () => {
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
  assertEquals(choices.map((choice) => choice.value), ["parent", "child"]);
  assertEquals(choices[1].hierarchyPath, ["Work", "Cowboy"]);
  assertEquals(sessionDirectoryChoices({ folders: [], placement: {} }), []);
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
