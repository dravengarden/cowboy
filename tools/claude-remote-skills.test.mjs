import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  derivedDescription,
  mirrorContent,
  skillFrontmatter,
  skillName,
  skillRoots,
  targetSkills,
  writeSkillPlugin,
} from "../plugins/claude-code/runtime/skills.mjs";
import {
  exposedCommand,
  nativeCommand,
  skillAllowlist,
} from "../plugins/claude-code/runtime/launch.mjs";
import {
  RUNTIME_ATTACHMENTS,
  skillAllows,
  targetSkillAppend,
  targetSkillListing,
  targetSkillShell,
  targetSkillText,
  validSkills,
} from "../plugins/claude-code/runtime/context-mod.js";

const marker = "cowboy" + "c".repeat(32);

test("skills are searched as native does: user, then the project up to its repository root", () => {
  assert.deepEqual(
    skillRoots("/w/repo/sub", "/home/u", "/w/repo").map((root) =>
      `${root.scope}:${root.kind}:${root.directory}`
    ),
    [
      "user:skill:/home/u/.claude/skills",
      "project:skill:/w/repo/sub/.claude/skills",
      "project:skill:/w/repo/.claude/skills",
      "user:command:/home/u/.claude/commands",
      "project:command:/w/repo/sub/.claude/commands",
      "project:command:/w/repo/.claude/commands",
    ],
  );
  // The home directory itself is never a project directory.
  assert.deepEqual(
    skillRoots("/home/u/p", "/home/u", undefined).map((root) => root.directory),
    [
      "/home/u/.claude/skills",
      "/home/u/p/.claude/skills",
      "/home/u/.claude/commands",
      "/home/u/p/.claude/commands",
    ],
  );
  assert.deepEqual(
    skillRoots("/srv/x", "/home/u", undefined).filter((root) =>
      root.scope === "project" && root.kind === "skill"
    ).map((root) => root.directory),
    ["/srv/x/.claude/skills", "/srv/.claude/skills", "/.claude/skills"],
  );
});

test("skill and command names follow native naming", () => {
  const skills = { kind: "skill", directory: "/p/.claude/skills" };
  const commands = { kind: "command", directory: "/p/.claude/commands" };
  assert.equal(
    skillName(skills, "/p/.claude/skills/review/SKILL.md"),
    "review",
  );
  assert.equal(skillName(skills, "/p/.claude/skills/a/b/SKILL.md"), undefined);
  assert.equal(
    skillName(skills, "/p/.claude/skills/.hidden/SKILL.md"),
    undefined,
  );
  assert.equal(
    skillName(commands, "/p/.claude/commands/grp/inner.md"),
    "grp:inner",
  );
  assert.equal(skillName(commands, "/p/.claude/commands/top.md"), "top");
  assert.equal(skillName(commands, "/p/.claude/commands/top.txt"), undefined);
  assert.equal(skillName(commands, "/other/top.md"), undefined);
  // A colon would read as a path separator in the plugin's layout.
  assert.equal(
    skillName(commands, "/p/.claude/commands/safe:..:..:escaped.md"),
    undefined,
  );
  assert.equal(skillName(commands, "/p/.claude/commands/a:b/c.md"), undefined);
});

test("precedence keeps the first of a name; skills native cannot run here are listed with a reason", () => {
  const file = (scope, kind, name, content) => ({
    scope,
    kind,
    name,
    path: `/${scope}/${name}/SKILL.md`,
    content,
  });
  const { entries, omitted } = targetSkills([
    file("user", "skill", "shared", "USER"),
    file("project", "skill", "shared", "PROJECT"),
    file("project", "skill", "hooked", "---\nhooks:\n  Stop: []\n---\nX"),
    file("project", "skill", "ps", "---\nshell: powershell\n---\nX"),
    file(
      "project",
      "skill",
      "tools",
      "---\nallowed-tools: Bash(git status:*), Read\n---\nX",
    ),
    file("user", "command", "shared", "COMMAND"),
  ]);
  assert.deepEqual(entries.map((entry) => [entry.name, entry.content]), [
    ["shared", "USER"],
    ["tools", "---\nallowed-tools: Bash(git status:*), Read\n---\nX"],
  ]);
  assert.deepEqual(entries[1].allowedTools, ["Bash(git status:*)", "Read"]);
  assert.deepEqual(omitted.map((entry) => entry.name), ["hooked", "ps"]);
  // Hooks under any key form native's YAML reading accepts are refused.
  for (
    const front of [
      '"hooks":\n  Stop: []',
      "'hooks': {}",
      '"ho\\u006fks": {}',
      "<<: {hooks: {}}",
      "? hooks\n: {}",
      "{hooks: {}}",
      "Hooks: {}",
    ]
  ) {
    assert.ok(skillFrontmatter(`---\n${front}\n---\nX`).hooks, front);
  }
  assert.ok(
    !skillFrontmatter("---\ndescription: Configure hooks: safely\n---\n").hooks,
  );
  // An unindented list belongs to its key.
  const unindented = skillFrontmatter(
    "---\nallowed-tools:\n- Bash(git status:*)\n- Read\n---\n",
  );
  assert.deepEqual(
    [unindented.hooks, unindented.allowedTools],
    [false, ["Bash(git status:*)", "Read"]],
  );
  assert.ok(skillFrontmatter("---\n- hooks\n---\n").hooks);
  // Top-level keys may share an indentation.
  const shifted = skillFrontmatter(
    "---\n  description: d\n  allowed-tools:\n    - Read\n---\n",
  );
  assert.deepEqual(
    [shifted.hooks, shifted.description, shifted.allowedTools],
    [false, "d", ["Read"]],
  );
  assert.equal(
    mirrorContent(
      {
        kind: "skill",
        path: "/t/s/SKILL.md",
        content: "---\n  name: n\n---\nB",
      },
      marker,
      "/t",
    ),
    '---\n  name: n\n  description: "B"\n---\nB',
  );
  assert.ok(skillFrontmatter("---\n  name: n\nhooks: {}\n---\n").hooks);
  // A comment grants nothing.
  assert.deepEqual(
    skillFrontmatter(
      "---\nallowed-tools: Read # Bash must ask\nother:\n  - x\n---\n",
    ).allowedTools,
    ["Read"],
  );
  assert.deepEqual(
    skillFrontmatter("---\nallowed-tools:\n  - Read  # Bash\n---\n")
      .allowedTools,
    ["Read"],
  );
  assert.deepEqual(
    skillFrontmatter("---\nallowed-tools:\n  - Bash(npm test)\n  - Edit\n---\n")
      .allowedTools,
    ["Bash(npm test)", "Edit"],
  );
});

test("the mirror names target directories and marks shell commands native would run", () => {
  const content = [
    "---",
    "description: d !`not a command`",
    "allowed-tools: Bash(${CLAUDE_SKILL_DIR}/run.sh)",
    "---",
    "Dir ${CLAUDE_SKILL_DIR} Project ${CLAUDE_PROJECT_DIR}",
    "Run: !`git status`",
    "Not: a!`x` and `code !`y` and [!`z`]",
    "```!",
    "echo one",
    "```",
    "",
  ].join("\n");
  const mirrored = mirrorContent(
    { kind: "skill", path: "/t/.claude/skills/s/SKILL.md", content },
    marker,
    "/t",
  );
  assert.equal(
    mirrored,
    [
      "---",
      "description: d !`not a command`",
      "allowed-tools: Bash(/t/.claude/skills/s/run.sh)",
      "---",
      "Dir /t/.claude/skills/s Project /t",
      `Run: !${marker}\`git status\``,
      "Not: a!`x` and `code !`y` and [!`z`]",
      "```" + marker + "!",
      "echo one",
      "```",
      "",
    ].join("\n"),
  );
  // Native lists a plugin's skill only with a description; the mirror gives
  // the one native derives for a local file without it.
  assert.equal(
    mirrorContent(
      { kind: "skill", path: "/t/s/SKILL.md", content: "\n# Title\nbody" },
      marker,
      "/t",
    ),
    '---\ndescription: "Title"\n---\n\n# Title\nbody',
  );
  assert.equal(
    mirrorContent(
      { kind: "skill", path: "/t/s/SKILL.md", content: "---\nname: n\n---\n" },
      marker,
      "/t",
    ),
    '---\nname: n\ndescription: "Skill"\n---\n',
  );
  assert.equal(
    mirrorContent(
      {
        kind: "command",
        path: "/t/c.md",
        content: "---\ndescription: ''\n---\nx",
      },
      marker,
      "/t",
    ),
    '---\ndescription: "x"\n---\nx',
  );
  assert.equal(
    derivedDescription("L".repeat(150), "skill"),
    "L".repeat(97) + "...",
  );
  assert.equal(derivedDescription("", "command"), "Custom command");
  // Commands have no skill directory.
  assert.equal(
    mirrorContent(
      { kind: "command", path: "/t/c.md", content: "${CLAUDE_SKILL_DIR}" },
      marker,
      "/t",
    ),
    '---\ndescription: "${CLAUDE_SKILL_DIR}"\n---\n${CLAUDE_SKILL_DIR}',
  );
});

test("the private plugin holds skills and commands under native's plugin layout", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-skill-plugin-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const plugin = join(root, "plugin");
  const skills = await writeSkillPlugin(
    plugin,
    targetSkills([
      {
        scope: "user",
        kind: "skill",
        name: "review",
        path: "/home/u/.claude/skills/review/SKILL.md",
        content: "R !`date`",
      },
      {
        scope: "project",
        kind: "command",
        name: "grp:inner",
        path: "/t/.claude/commands/grp/inner.md",
        content: "G",
      },
    ]),
    marker,
    "/t",
  );
  assert.equal(
    await readFile(join(plugin, "skills", "review", "SKILL.md"), "utf8"),
    `---\ndescription: "R !\`date\`"\n---\nR !${marker}\`date\``,
  );
  assert.equal(
    await readFile(join(plugin, "commands", "grp", "inner.md"), "utf8"),
    '---\ndescription: "G"\n---\nG',
  );
  assert.deepEqual(
    JSON.parse(
      await readFile(join(plugin, ".claude-plugin", "plugin.json"), "utf8"),
    ),
    { name: "cowboy-target", version: "1.0.0" },
  );
  assert.equal(
    skills.entries[0].mirrorDirectory,
    join(plugin, "skills", "review"),
  );
  assert.equal(
    skills.entries[0].targetDirectory,
    "/home/u/.claude/skills/review",
  );
  assert.ok(validSkills(skills));
  assert.ok(!validSkills({ ...skills, marker: "guessable" }));
});

const skills = {
  prefix: "cowboy-target:",
  marker,
  entries: [
    {
      name: "proj",
      kind: "skill",
      scope: "project",
      mirrorDirectory: "/stage/target-skills/skills/proj",
      targetDirectory: "/t/.claude/skills/proj",
      allowedTools: [],
    },
    {
      name: "simplify",
      kind: "skill",
      scope: "user",
      mirrorDirectory: "/stage/target-skills/skills/simplify",
      targetDirectory: "/home/u/.claude/skills/simplify",
      allowedTools: [],
    },
    {
      name: "grp:inner",
      kind: "command",
      scope: "project",
      allowedTools: [],
    },
  ],
  omitted: [],
  bundled: ["simplify", "init"],
  unavailable: { dataviz: "files" },
};

test("mirror directories map whole, the longest name first", () => {
  const nested = {
    prefix: "cowboy-target:",
    entries: [
      {
        name: "foo",
        mirrorDirectory: "/stage/skills/foo",
        targetDirectory: "/home/u/.claude/skills/foo",
      },
      {
        name: "foo-bar",
        mirrorDirectory: "/stage/skills/foo-bar",
        targetDirectory: "/p/.claude/skills/foo-bar",
      },
    ],
  };
  assert.equal(
    targetSkillText(
      "/stage/skills/foo-bar/x and /stage/skills/foo/y",
      nested,
    ),
    "/p/.claude/skills/foo-bar/x and /home/u/.claude/skills/foo/y",
  );
});

test("the skill listing shows target skills by native name and order", () => {
  const listing = [
    "The following skills are available for use with the Skill tool:\n",
    "- cowboy-target:grp:inner: grouped",
    "- cowboy-target:proj: project skill",
    "- cowboy-target:simplify: user simplify",
    "- simplify: bundled simplify",
    "- init: bundled init\nsecond line",
  ].join("\n");
  assert.equal(
    targetSkillListing(listing, skills),
    [
      "The following skills are available for use with the Skill tool:\n",
      "- simplify: user simplify",
      "- proj: project skill",
      "- grp:inner: grouped",
      "- init: bundled init\nsecond line",
    ].join("\n"),
  );
  // Native renders the stored listing again; a projected one is kept.
  const projected = targetSkillListing(listing, skills);
  assert.equal(targetSkillListing(projected, skills), projected);
  assert.equal(
    targetSkillListing(
      "The following skills:\n\n- init: bundled\n- cowboy-target:proj",
      skills,
    ),
    "The following skills:\n\n- proj\n- init: bundled",
  );
  assert.equal(
    targetSkillText(
      "Base directory for this skill: /stage/target-skills/skills/proj\n\nLaunching skill: cowboy-target:proj",
      skills,
    ),
    "Base directory for this skill: /t/.claude/skills/proj\n\nLaunching skill: proj",
  );
  for (
    const type of ["file", "directory", "nested_memory", "edited_text_file"]
  ) {
    assert.ok(RUNTIME_ATTACHMENTS.has(type), type);
  }
  for (const type of ["skill_listing", "todo_reminder", "task_status"]) {
    assert.ok(!RUNTIME_ATTACHMENTS.has(type), type);
  }
});

test("commands are shown and typed by the names a local session uses", () => {
  assert.equal(exposedCommand("cowboy-target:proj", skills), "proj");
  assert.equal(exposedCommand("cowboy-target:gone", skills), undefined);
  // The bundled skill a target skill shadows, and refused local commands.
  assert.equal(exposedCommand("simplify", skills), undefined);
  assert.equal(exposedCommand("init", skills), "init");
  assert.equal(exposedCommand("compact", skills), "compact");
  assert.equal(exposedCommand("doctor", skills), undefined);
  assert.equal(
    exposedCommand("mcp__matrix__prompt", skills),
    "mcp__matrix__prompt",
  );
  assert.deepEqual(skillAllowlist(skills), [
    "cowboy-target:proj",
    "cowboy-target:simplify",
    "cowboy-target:grp:inner",
    "init",
  ]);
  assert.equal(nativeCommand("/proj now", skills), "cowboy-target:proj");
  assert.equal(nativeCommand("/grp:inner", skills), "cowboy-target:grp:inner");
  assert.equal(nativeCommand("/init", skills), "init");
  assert.equal(nativeCommand("plain text", skills), undefined);
  // A path is an ordinary prompt, as is a command native names otherwise.
  assert.equal(nativeCommand("/home/u/file what is this", skills), undefined);
  assert.equal(nativeCommand("/mcp__matrix__prompt", skills), undefined);
  assert.throws(() => nativeCommand("/doctor", skills), /unavailable/);
  // A target skill that cannot run here still shadows the bundled one.
  const shadowed = { ...skills, omitted: [{ name: "init", reason: "hooks" }] };
  assert.equal(exposedCommand("init", shadowed), undefined);
  assert.ok(!skillAllowlist(shadowed).includes("init"));
  assert.throws(() => nativeCommand("/init", shadowed), /unavailable/);
});

test("skill commands run on the target in order and fail the skill as natively", async () => {
  const runs = [];
  const run = async (command, raw) => {
    runs.push([command, raw]);
    return command === "false"
      ? { failure: `Shell command failed for pattern "${raw}":` }
      : { text: `out:${command}` };
  };
  const text =
    `A !${marker}\`echo 1\` B\n\`\`\`${marker}!\necho 2\n\`\`\`\nC !${marker}\`$1\``;
  assert.deepEqual(await targetSkillShell(text, marker, run), {
    text: "A out:echo 1 B\nout:echo 2\nC out:$1",
  });
  assert.deepEqual(runs.map(([command]) => command), [
    "echo 2",
    "echo 1",
    "$1",
  ]);
  assert.equal(runs[1][1], "!`echo 1`");
  assert.deepEqual(
    await targetSkillShell(`x !${marker}\`false\` y`, marker, run),
    { failure: 'Shell command failed for pattern "!`false`":' },
  );
  // Text without this session's marker is never run.
  runs.length = 0;
  assert.deepEqual(
    await targetSkillShell("plain !`echo no`", marker, run),
    { text: "plain !`echo no`" },
  );
  assert.equal(runs.length, 0);
});

test("allowed-tools grant only single plain commands", () => {
  const rules = ["Bash(git status:*)", "Bash(npm test)", "Read"];
  assert.ok(skillAllows("git status", rules));
  assert.ok(skillAllows("git status --short", rules));
  assert.ok(skillAllows("npm test", rules));
  assert.ok(!skillAllows("npm test --watch", rules));
  assert.ok(!skillAllows("git statusx", rules));
  assert.ok(!skillAllows("git status && rm -rf x", rules));
  assert.ok(!skillAllows("git status $(rm x)", rules));
  assert.ok(skillAllows("anything", ["Bash"]));
  assert.ok(!skillAllows("anything", ["Read"]));
});

test("messages about target skills enter the session under native names", () => {
  const event = {
    door: "tool-result",
    origin: { kind: "tool", tool: "Skill" },
    message: {
      role: "user",
      content: [
        {
          type: "tool_result",
          tool_use_id: "t",
          content: "Launching skill: cowboy-target:proj",
        },
        {
          type: "text",
          text:
            "Base directory for this skill: /stage/target-skills/skills/proj\n\nX",
        },
      ],
    },
  };
  assert.deepEqual(targetSkillAppend(event, skills).message.content, [
    { type: "tool_result", tool_use_id: "t", content: "Launching skill: proj" },
    {
      type: "text",
      text: "Base directory for this skill: /t/.claude/skills/proj\n\nX",
    },
  ]);
  // A stored listing is ordered as it renders, inside its reminder.
  const stored = {
    door: "attachment",
    message: {
      name: "skill_listing",
      content: [{
        type: "text",
        text:
          "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n- cowboy-target:grp:inner: grouped\n- cowboy-target:proj: project skill\n- simplify: bundled\n</system-reminder>",
      }],
    },
  };
  assert.equal(
    targetSkillAppend(stored, skills).message.content[0].text,
    "<system-reminder>\nThe following skills are available for use with the Skill tool:\n\n- proj: project skill\n- grp:inner: grouped\n- simplify: bundled\n</system-reminder>",
  );
  // Only skill messages are projected: a file that names the prefix keeps it.
  const file = {
    door: "tool-result",
    origin: { kind: "tool", tool: "Read" },
    message: { content: [{ type: "text", text: "cowboy-target:proj" }] },
  };
  assert.equal(targetSkillAppend(file, skills), file);
  const typed = {
    door: "command",
    origin: { kind: "unclassified" },
    message: {
      content: [{
        type: "text",
        text:
          "<command-message>cowboy-target:grp:inner</command-message>\n<command-name>/cowboy-target:grp:inner</command-name>\n<command-args>keep cowboy-target:x</command-args>",
      }],
    },
  };
  assert.equal(
    targetSkillAppend(typed, skills).message.content[0].text,
    "<command-message>grp:inner</command-message>\n<command-name>/grp:inner</command-name>\n<command-args>keep cowboy-target:x</command-args>",
  );
  const plain = { door: "prompt", message: { content: "nothing to rename" } };
  assert.equal(targetSkillAppend(plain, skills), plain);
});

test("a skill's rules name its target directory, as native reads them", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-skill-rules-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const skills = await writeSkillPlugin(
    join(root, "plugin"),
    targetSkills([{
      scope: "project",
      kind: "skill",
      name: "run",
      path: "/t/.claude/skills/run/SKILL.md",
      content:
        "---\nallowed-tools: Bash(${CLAUDE_SKILL_DIR}/run.sh)\n---\n!`${CLAUDE_SKILL_DIR}/run.sh`",
    }]),
    marker,
    "/t",
  );
  assert.deepEqual(skills.entries[0].allowedTools, [
    "Bash(/t/.claude/skills/run/run.sh)",
  ]);
  assert.ok(
    skillAllows("/t/.claude/skills/run/run.sh", skills.entries[0].allowedTools),
  );
});
