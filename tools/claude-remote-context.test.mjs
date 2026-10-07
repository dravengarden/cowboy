import assert from "node:assert/strict";
import test from "node:test";
import {
  globMatches,
  importsOf,
  instructionFiles,
  nestedInstructions,
  ruleScope,
} from "../plugins/claude-code/runtime/instructions.mjs";

// The tree of the native-local baseline (2.1.287), under a target root.
const tree = {
  "/home/u/.claude/CLAUDE.md": "USER_CLAUDE_MD\n",
  "/w/parent/CLAUDE.md": "PARENT_CLAUDE_MD\n",
  "/w/parent/AGENTS.md": "PARENT_AGENTS_MD\n",
  "/w/parent/project/CLAUDE.md": "ROOT_CLAUDE_MD\n@docs/imported.md\n",
  "/w/parent/project/docs/imported.md": "IMPORTED_BY_AT\n",
  "/w/parent/project/CLAUDE.local.md": "ROOT_CLAUDE_LOCAL_MD\n",
  "/w/parent/project/.claude/CLAUDE.md": "DOT_CLAUDE_CLAUDE_MD\n",
  "/w/parent/project/.claude/rules/always.md": "RULE_ALWAYS\n",
  "/w/parent/project/.claude/rules/scoped.md":
    '---\npaths:\n  - "sub/**"\n---\nRULE_SCOPED_SUB\n',
  "/w/parent/project/AGENTS.md": "ROOT_AGENTS_MD\n",
  "/w/parent/project/sub/CLAUDE.md": "NESTED_SUB_CLAUDE_MD\n",
  "/w/parent/project/sub/AGENTS.md": "NESTED_SUB_AGENTS_MD\n",
  "/w/parent/project/sub3/CLAUDE.md": "NESTED_SUB3_CLAUDE_MD\n",
  "/w/parent/other/CLAUDE.md": "OUTSIDE_OTHER_CLAUDE_MD\n",
};
const io = {
  read: async (path) => tree[path],
  rules: async (directory) =>
    Object.keys(tree).filter((path) => path.startsWith(directory + "/"))
      .sort(),
};

test("instruction files follow native's discovery and order", async () => {
  const { files, conditional } = await instructionFiles({
    cwd: "/w/parent/project",
    home: "/home/u",
    ...io,
  });
  assert.deepEqual(
    files.map(({ path, kind, parent }) => [path, kind, parent]),
    [
      ["/home/u/.claude/CLAUDE.md", "user", undefined],
      ["/w/parent/CLAUDE.md", "project", undefined],
      ["/w/parent/project/CLAUDE.md", "project", undefined],
      [
        "/w/parent/project/docs/imported.md",
        "project",
        "/w/parent/project/CLAUDE.md",
      ],
      ["/w/parent/project/.claude/CLAUDE.md", "project", undefined],
      ["/w/parent/project/.claude/rules/always.md", "project", undefined],
      ["/w/parent/project/CLAUDE.local.md", "local", undefined],
    ],
  );
  assert.deepEqual(conditional.map((rule) => rule.path), [
    "/w/parent/project/.claude/rules/scoped.md",
  ]);
});

test("a Read brings nested files and matching rules once, as natively", async () => {
  const { files, conditional } = await instructionFiles({
    cwd: "/w/parent/project",
    home: "/home/u",
    ...io,
  });
  const attached = new Set(files.map((file) => file.path));
  const nested = (path) =>
    nestedInstructions({
      cwd: "/w/parent/project",
      path,
      read: io.read,
      conditional,
      attached,
    });
  assert.deepEqual(await nested("/w/parent/project/sub/file.txt"), [
    {
      path: "/w/parent/project/sub/CLAUDE.md",
      content: "NESTED_SUB_CLAUDE_MD\n",
    },
    {
      path: "/w/parent/project/.claude/rules/scoped.md",
      content: "RULE_SCOPED_SUB\n",
    },
  ]);
  assert.deepEqual(await nested("/w/parent/project/sub/file.txt"), []);
  assert.deepEqual(
    (await nested("/w/parent/project/sub3/e.txt")).map((file) => file.path),
    ["/w/parent/project/sub3/CLAUDE.md"],
  );
  assert.deepEqual(await nested("/w/parent/other/f.txt"), []);
  assert.deepEqual(await nested("/w/parent/project/top.txt"), []);
});

test("rule scopes, globs and imports", () => {
  assert.deepEqual(ruleScope("---\npaths: [\"a/**\", 'b/*.ts']\n---\nX\n"), {
    content: "X\n",
    paths: ["a/**", "b/*.ts"],
  });
  assert.deepEqual(ruleScope("no frontmatter\n").paths, []);
  assert.ok(globMatches("sub/**", "sub/a/b.txt"));
  assert.ok(globMatches("**/*.ts", "x.ts"));
  assert.ok(globMatches("**/*.ts", "a/b/x.ts"));
  assert.ok(!globMatches("src/*.ts", "src/a/x.ts"));
  assert.deepEqual(
    importsOf(
      "see @docs/a.md and @~/notes.md\n`@not/code.md`\n```\n@fenced.md\n```\nmail a@b.c",
      "/p/CLAUDE.md",
      "/home/u",
    ),
    ["/p/docs/a.md", "/home/u/notes.md"],
  );
});

test("native's Git section describes the target", async () => {
  const { targetSessionContext } = await import(
    "../plugins/claude-code/runtime/context-mod.js?session-context"
  );
  const native =
    "As you answer the user's questions, you can use the following context:\n# gitStatus\nThis is the git status at the start of the conversation. Runtime.\n\nStatus:\n(clean)\n\nClaude Code attached this context automatically; it isn't part of the user's message.";
  assert.equal(
    targetSessionContext(
      native,
      "This is the git status at the start of the conversation. Target.",
    ),
    "As you answer the user's questions, you can use the following context:\n# gitStatus\nThis is the git status at the start of the conversation. Target.\n\nClaude Code attached this context automatically; it isn't part of the user's message.",
  );
  assert.equal(targetSessionContext(native, null), null);
  const withEmail = native.replace(
    "# gitStatus",
    "# userEmail\nu@example.com\n# gitStatus",
  );
  assert.equal(
    targetSessionContext(withEmail, null),
    "As you answer the user's questions, you can use the following context:\n# userEmail\nu@example.com\n\n\nClaude Code attached this context automatically; it isn't part of the user's message.",
  );
});

test("the module's tool context reads without the chain's label", async () => {
  const { unlabeledContext } = await import(
    "../plugins/claude-code/runtime/context-mod.js?unlabeled"
  );
  assert.equal(
    unlabeledContext(
      "tool.call hook additional context: PostToolUse:Write hook additional context: X",
    ),
    "PostToolUse:Write hook additional context: X",
  );
  assert.equal(unlabeledContext("other"), "other");
});

test("rule globs keep braces, classes and quoted commas", () => {
  assert.deepEqual(
    ruleScope("---\npaths: [\"src/**/*.{ts,tsx}\", 'docs/[ab]*.md']\n---\nX")
      .paths,
    ["src/**/*.{ts,tsx}", "docs/[ab]*.md"],
  );
  assert.ok(globMatches("src/**/*.{ts,tsx}", "src/a/b.tsx"));
  assert.ok(globMatches("src/**/*.{ts,tsx}", "src/x.ts"));
  assert.ok(!globMatches("src/**/*.{ts,tsx}", "src/x.js"));
  assert.ok(globMatches("docs/[ab]*.md", "docs/alpha.md"));
  assert.ok(!globMatches("docs/[ab]*.md", "docs/gamma.md"));
  assert.ok(globMatches("docs/[!ab]*.md", "docs/gamma.md"));
});

test("a nested file's imports follow it, once", async () => {
  const files = {
    "/p/sub/CLAUDE.md": "SUB\n@../shared.md\n",
    "/p/shared.md": "SHARED\n",
  };
  const attached = new Set();
  const nested = await nestedInstructions({
    cwd: "/p",
    path: "/p/sub/x.txt",
    read: async (path) => files[path],
    conditional: [],
    attached,
  });
  assert.deepEqual(nested.map((file) => file.path), [
    "/p/sub/CLAUDE.md",
    "/p/shared.md",
  ]);
});

test("the target's Git section is added when native had none", async () => {
  const { targetSessionContext } = await import(
    "../plugins/claude-code/runtime/context-mod.js?session-context-add"
  );
  assert.equal(
    targetSessionContext(
      "As you answer the user's questions, you can use the following context:\n# userEmail\nu@example.com\n\nClaude Code attached this context automatically.",
      "GIT",
    ),
    "As you answer the user's questions, you can use the following context:\n# userEmail\nu@example.com\n# gitStatus\nGIT\n\nClaude Code attached this context automatically.",
  );
});

test("the user's scoped rules match project-relative paths", async () => {
  const files = {
    "/home/u/.claude/rules/ts.md":
      '---\npaths: ["src/**"]\n---\nUSER_TS_RULE\n',
  };
  const { conditional } = await instructionFiles({
    cwd: "/work/p",
    home: "/home/u",
    read: async (path) => files[path],
    rules: async (directory) =>
      Object.keys(files).filter((path) => path.startsWith(directory + "/")),
  });
  const nested = await nestedInstructions({
    cwd: "/work/p",
    path: "/work/p/src/a.ts",
    read: async (path) => files[path],
    conditional,
    attached: new Set(),
  });
  assert.deepEqual(nested.map((file) => file.content), ["USER_TS_RULE\n"]);
});

test("multi-line flow sequences and comments in rule paths", () => {
  assert.deepEqual(
    ruleScope(
      '---\npaths: [\n  "src/**",  # sources\n  "lib/*.ts"\n]\nother: x\n---\nX',
    ).paths,
    ["src/**", "lib/*.ts"],
  );
  assert.deepEqual(
    ruleScope('---\npaths:\n  - "a/**" # tail comment\n  - "b#c/*"\n---\nX')
      .paths,
    ["a/**", "b#c/*"],
  );
});

test("a failed nested read leaves its files for a later Read", async () => {
  let broken = true;
  const files = { "/p/s/CLAUDE.md": "S\n@imp.md\n", "/p/s/imp.md": "I\n" };
  const read = async (path) => {
    if (broken && path === "/p/s/imp.md") throw new Error("transport");
    return files[path];
  };
  const attached = new Set();
  const args = { cwd: "/p", path: "/p/s/x", read, conditional: [], attached };
  await assert.rejects(nestedInstructions(args));
  assert.equal(attached.size, 0);
  broken = false;
  assert.deepEqual((await nestedInstructions(args)).map((file) => file.path), [
    "/p/s/CLAUDE.md",
    "/p/s/imp.md",
  ]);
});

test("a quoted character class does not close a flow sequence", () => {
  assert.deepEqual(
    ruleScope('---\npaths: [\n  "src/[ab]/*.ts",\n  "lib/**"\n]\n---\nX').paths,
    ["src/[ab]/*.ts", "lib/**"],
  );
});

test("a user rule under a home that contains the project is collected once", async () => {
  const files = {
    "/home/u/.claude/rules/p.md":
      '---\npaths: ["projects/**"]\n---\nUSER_RULE\n',
  };
  const { conditional } = await instructionFiles({
    cwd: "/home/u/projects/app",
    home: "/home/u",
    read: async (path) => files[path],
    rules: async (directory) =>
      Object.keys(files).filter((path) => path.startsWith(directory + "/")),
  });
  assert.deepEqual(conditional.map((rule) => rule.root), [
    "/home/u/projects/app",
  ]);
  const nested = await nestedInstructions({
    cwd: "/home/u/projects/app",
    path: "/home/u/projects/app/README.md",
    read: async (path) => files[path],
    conditional,
    attached: new Set(),
  });
  assert.deepEqual(nested, []);
});
