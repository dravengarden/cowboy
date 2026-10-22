import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { SessionMeta } from "./protocol";
import type { SessionFoldersValue } from "./sessionFolders";
import { folderSessionHints } from "./folderSessionHints";
import {
  createDefaultsChanges,
  createDefaultsSummary,
} from "./createDefaultsSummary";

const tree: SessionFoldersValue = {
  folders: [
    { id: "suger", name: "Suger", parent: null, position: 0, project: null },
    { id: "fwk", name: "CS-FWK-001", parent: "suger", position: 0, project: null },
    { id: "atl", name: "CS-ATL-006", parent: "suger", position: 1, project: null },
    { id: "empty", name: "New", parent: "suger", position: 2, project: null },
  ],
  placement: {
    "sess-100": "fwk",
    "sess-300": "atl",
    "sess-200": "suger",
    "sess-400": "fwk",
  },
};

function session(id: string, extra: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id,
    title: id,
    provider: "claude-code",
    provider_account_id: "lynx",
    machine_id: "ovh",
    workspace_id: "suger",
    cwd: "/runtime/" + id,
    status: "idle",
    ...extra,
  };
}

test("a directory follows its own newest session before its subtree", () => {
  const hints = folderSessionHints(
    [session("sess-100"), session("sess-300", { workspace_id: "other" })],
    tree,
    "fwk",
  );
  assertEquals(hints?.project, "suger");
  assertEquals(hints?.provider, "claude-code");
  assertEquals(hints?.runtime_machine, "ovh");
  assertEquals(hints?.account, "lynx");
  assertEquals(hints?.folder, "fwk");
});

test("an empty directory follows the newest session of its nearest ancestor", () => {
  const hints = folderSessionHints(
    [session("sess-100"), session("sess-300", { workspace_id: "other" })],
    tree,
    "empty",
  );
  assertEquals(hints?.project, "other");
  assertEquals(hints?.folder, "suger");
});

test("trashed, system and unplaced sessions never supply hints", () => {
  assertEquals(
    folderSessionHints(
      [
        session("sess-400", { trashed_at_ms: 1 }),
        session("sess-100", { system: true }),
        session("sess-999"),
      ],
      tree,
      "fwk",
    ),
    undefined,
  );
  assertEquals(folderSessionHints([session("sess-100")], tree, null), undefined);
});

test("the summary names its real sources", () => {
  assertEquals(
    createDefaultsSummary({ directoryName: "X", explicitSources: [] }),
    "X has no defaults or sessions yet · using this device's defaults",
  );
  assertEquals(
    createDefaultsSummary({
      directoryName: "X",
      explicitSources: [],
      hintSession: "T",
    }),
    "Choices match the latest session “T” · Change any choice for this session.",
  );
  assertEquals(
    createDefaultsSummary({
      directoryName: "X",
      explicitSources: ["Suger", "Suger"],
      hintSession: "T",
    }),
    "Defaults from Suger · other choices match the latest session “T” · Change any choice for this session.",
  );
});

const chosen = {
  project: "suger",
  projectName: "suger",
  machine: "hawk",
  runtime_machine: "ovh",
  provider: "claude-code",
};

test("saving defaults pins only what the directory does not already pin", () => {
  assertEquals(createDefaultsChanges({}, chosen), {
    project: "suger",
    machine: "hawk",
    runtime_machine: "ovh",
    provider: "claude-code",
  });
  assertEquals(
    createDefaultsChanges({ project: "suger", provider: "claude-code" }, chosen),
    undefined,
  );
  assertEquals(
    createDefaultsChanges(
      {
        project: "suger",
        runtime_machine: "ovh",
        provider: "codex",
        preset: "codex/astra",
      },
      chosen,
    ),
    { provider: "claude-code", preset: null },
  );
});
