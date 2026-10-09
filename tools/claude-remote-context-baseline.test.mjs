// The session context the plugin builds for the target, checked against
// native-local observations (tools/claude_context_native_baseline.json,
// written by tools/claude_context_native_probe.py).
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  instructionFiles,
  nestedInstructions,
} from "../plugins/claude-code/runtime/instructions.mjs";
import { WorkspaceTools } from "../plugins/claude-code/runtime/tools.mjs";
import { unlabeledContext } from "../plugins/claude-code/runtime/context-mod.js";

const baseline = JSON.parse(
  readFileSync(
    new URL("./claude_context_native_baseline.json", import.meta.url),
    "utf8",
  ),
);

// The probe's tree, under a fixed root (tools/claude_context_native_probe.py).
const FILES = {
  "CLAUDE.md": "ROOT_CLAUDE_MD\n@docs/imported.md\n",
  "docs/imported.md": "IMPORTED_BY_AT\n",
  "CLAUDE.local.md": "ROOT_CLAUDE_LOCAL_MD\n",
  ".claude/CLAUDE.md": "DOT_CLAUDE_CLAUDE_MD\n",
  ".claude/rules/always.md": "RULE_ALWAYS\n",
  ".claude/rules/scoped.md":
    '---\npaths:\n  - "sub/**"\n---\nRULE_SCOPED_SUB\n',
  "AGENTS.md": "ROOT_AGENTS_MD\n",
  "sub/CLAUDE.md": "NESTED_SUB_CLAUDE_MD\n",
  "sub/AGENTS.md": "NESTED_SUB_AGENTS_MD\n",
  "sub2/CLAUDE.md": "NESTED_SUB2_CLAUDE_MD\n",
  "sub3/CLAUDE.md": "NESTED_SUB3_CLAUDE_MD\n",
};
const AGENTS_ONLY = {
  "AGENTS.md": "ROOT_AGENTS_MD\n",
  "sub/AGENTS.md": "NESTED_SUB_AGENTS_MD\n",
};
const project = "/r/parent/project";

function io(files) {
  const tree = {
    "/r/home/.claude/CLAUDE.md": "USER_CLAUDE_MD\n",
    "/r/parent/CLAUDE.md": "PARENT_CLAUDE_MD\n",
    "/r/parent/AGENTS.md": "PARENT_AGENTS_MD\n",
    "/r/parent/other/CLAUDE.md": "OUTSIDE_OTHER_CLAUDE_MD\n",
    ...Object.fromEntries(
      Object.entries(files).map(([name, text]) => [`${project}/${name}`, text]),
    ),
  };
  return {
    read: async (path) => tree[path],
    rules: async (directory) =>
      Object.keys(tree).filter((path) => path.startsWith(directory + "/"))
        .sort(),
  };
}

const markers = (files) =>
  files.flatMap(({ content }) => content.split("\n"))
    .filter((line) => /^[A-Z0-9_]+$/.test(line) && line.includes("_"))
    .sort();

// The probe's steps; a Write attaches nothing natively.
const STEPS = {
  "read sub/file.txt": `${project}/sub/file.txt`,
  "read sub/file.txt again": `${project}/sub/file.txt`,
  "write sub2/new.txt": null,
  "read sub3/e.txt": `${project}/sub3/e.txt`,
  "read ../other/f.txt": "/r/parent/other/f.txt",
};

for (
  const [name, files] of [["discovery", FILES], ["agents_only", AGENTS_ONLY]]
) {
  test(`${name}: instruction files load and attach as natively`, async () => {
    const { read, rules } = io(files);
    const { files: initial, conditional } = await instructionFiles({
      cwd: project,
      home: "/r/home",
      read,
      rules,
    });
    assert.deepEqual(markers(initial), baseline[name].start);
    const attached = new Set(initial.map((file) => file.path));
    for (const [step, path] of Object.entries(STEPS)) {
      const nested = path === null ? [] : await nestedInstructions({
        cwd: project,
        path,
        read,
        conditional,
        attached,
      });
      assert.deepEqual(markers(nested), baseline[name].after[step], step);
    }
  });
}

const replies = (answers) => ({
  command: async (argv) => {
    const key = argv.slice(2).join(" ");
    return answers[key] ?? { exitCode: 1, output: "" };
  },
});

test("the Git section reads as native's inside a repository", async () => {
  const git = await WorkspaceTools.prototype.gitStatus.call(replies({
    "rev-parse --is-inside-work-tree": { exitCode: 0, output: "true\n" },
    "branch --show-current": { exitCode: 0, output: "main\n" },
    "rev-parse --verify --quiet refs/heads/main": { exitCode: 0, output: "x" },
    "status --short": { exitCode: 0, output: "?? b.txt\n" },
    "log --oneline -n 5": { exitCode: 0, output: "COMMIT first\n" },
  }));
  assert.deepEqual(git.split("\n"), baseline.git_repository.git_status_lines);
});

test("outside a repository native has no Git section", async () => {
  const observed = baseline.no_repository;
  assert.equal(observed.runtime_is_repository, false);
  assert.equal(observed.git_status_header, false);
  assert.equal(
    await WorkspaceTools.prototype.gitStatus.call(replies({})),
    undefined,
  );
});

test("the environment block has native's lines", () => {
  // tools.mjs writes the same lines with the target's facts.
  const source = readFileSync(
    new URL("../plugins/claude-code/runtime/tools.mjs", import.meta.url),
    "utf8",
  );
  for (
    const [name, observed] of [
      ["git_repository", "true"],
      ["no_repository", "false"],
    ]
  ) {
    const lines = baseline[name].environment_lines;
    assert.deepEqual(lines, [
      "You have been invoked in the following environment: ",
      " - Primary working directory: <ROOT>/runtime",
      ` - Is a git repository: ${observed}`,
      " - Platform: linux",
      " - Shell: SHELL",
      " - OS Version: OS",
    ]);
  }
  assert.ok(
    source.includes(
      "# Environment\\nYou have been invoked in the following environment: \\n - Primary working directory: ${this.cwd}\\n - Is a git repository: ${",
    ),
  );
  assert.ok(source.includes("}\\n - Platform: linux\\n - Shell: ${"));
  assert.ok(source.includes("}\\n - OS Version: ${"));
});

test("native labels a Mod's tool context, and the plugin removes the label", () => {
  assert.equal(baseline.tool_context_label.labeled, true);
  assert.equal(
    unlabeledContext("tool.call hook additional context: MOD_TOOL_CONTEXT"),
    "MOD_TOOL_CONTEXT",
  );
});
