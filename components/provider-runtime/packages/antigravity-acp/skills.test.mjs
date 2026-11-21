import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { Connection } from "../../execution-connection.mjs";
import { Facade } from "./facade.mjs";
import { buildProjection } from "./projection.mjs";
import {
  discoverSkills,
  expandSkillCommand,
  parseFrontmatter,
  SKILL_LIMITS,
  skillCommands,
  skillsSection,
} from "./skills.mjs";
import { fakeEndpoint } from "./test-endpoint.mjs";

async function fixture(t) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-skills-")));
  const root = join(base, "workspace");
  mkdirSync(join(root, "sub"), { recursive: true });
  const socket = join(base, "exec.sock");
  const endpoint = await fakeEndpoint(socket, root, {});
  const connection = await Connection.open(
    {
      schema: 1,
      endpoint: `unix:${socket}`,
      bearer_token: "a".repeat(64),
      binding: { schema: 1, environment: { id: "env" }, workspace: { cwd: root } },
    },
    { clientName: "cowboy-antigravity-test", requestTimeoutMs: 20000 },
  );
  const facade = await Facade.create(connection, {});
  t.after(() => {
    connection.close();
    endpoint.close();
    rmSync(base, { recursive: true, force: true });
  });
  return { base, root, facade };
}

function skill(directory, name, frontmatter) {
  mkdirSync(join(directory, name), { recursive: true });
  writeFileSync(join(directory, name, "SKILL.md"), `${frontmatter}\n\n# Body\nSECRET_BODY\n`);
}

test("frontmatter yields a name and a one-line description", () => {
  assert.deepEqual(parseFrontmatter("---\nname: deploy\ndescription: Ship it.\n---\nbody"), {
    name: "deploy",
    description: "Ship it.",
  });
  assert.deepEqual(
    parseFrontmatter('---\nname: "a-b"\ndescription: >\n  Folded\n  text.\nother: x\n---\n'),
    { name: "a-b", description: "Folded text." },
  );
  assert.equal(parseFrontmatter("name: x\n"), undefined, "no frontmatter");
  assert.equal(parseFrontmatter("---\nname: bad name\n---\n"), undefined, "invalid name");
  assert.equal(parseFrontmatter("---\nname: open\n"), undefined, "unterminated");
  const long = parseFrontmatter(`---\nname: n\ndescription: ${"d".repeat(2000)}\n---\n`);
  assert.equal(long.description.length, SKILL_LIMITS.description);
});

test("skills are read from the target; the nearest definition wins and symlinks are skipped", async (t) => {
  const { base, root, facade } = await fixture(t);
  skill(
    join(root, ".agents", "skills"),
    "deploy",
    "---\nname: deploy\ndescription: Root deploy\n---",
  );
  skill(join(root, ".agents", "skills"), "lint", "---\nname: lint\ndescription: Lint\n---");
  skill(
    join(root, "sub", ".claude", "skills"),
    "deploy",
    "---\nname: deploy\ndescription: Sub deploy\n---",
  );
  mkdirSync(join(root, ".agents", "skills", "empty"));
  // `.claude/skills` -> `.agents/skills` at the root is read once, under its target.
  mkdirSync(join(root, ".claude"));
  symlinkSync(join(root, ".agents", "skills"), join(root, ".claude", "skills"));
  // A skill file escaping the workspace is skipped.
  mkdirSync(join(base, "outside", "evil"), { recursive: true });
  writeFileSync(join(base, "outside", "evil", "SKILL.md"), "---\nname: evil\n---\n");
  symlinkSync(join(base, "outside", "evil"), join(root, ".agents", "skills", "evil"));

  const skills = await discoverSkills(facade, [root, join(root, "sub")], root);
  // The projection walks from the repository root down to the directory.
  execFileSync("git", ["-C", root, "init", "-q", "-b", "main"], { stdio: "pipe" });
  assert.deepEqual(skills, [
    {
      name: "deploy",
      description: "Sub deploy",
      path: join(root, "sub", ".claude", "skills", "deploy", "SKILL.md"),
    },
    {
      name: "lint",
      description: "Lint",
      path: join(root, ".agents", "skills", "lint", "SKILL.md"),
    },
  ]);

  let reported;
  const projection = await buildProjection(facade, join(root, "sub"), {
    onSkills: (value) => {
      reported = value;
    },
  });
  assert.deepEqual(reported, skills);
  assert.match(projection, /<repository_skills>/);
  assert.match(projection, /- deploy: Sub deploy \(/);
  assert.ok(!projection.includes("SECRET_BODY"), "only the frontmatter is projected");
  const fence = /(COWBOY_UNTRUSTED_[a-f0-9]{32})/.exec(projection)[1];
  const listed = projection.slice(projection.indexOf("<repository_skills>"));
  // The intro names the fence once; the list sits between two more markers.
  assert.equal(listed.split(fence).length, 4, "the list is fenced");
});

test("a repository without skills adds no section", async (t) => {
  const { root, facade } = await fixture(t);
  let reported;
  const projection = await buildProjection(facade, root, {
    onSkills: (value) => {
      reported = value;
    },
  });
  assert.deepEqual(reported, []);
  assert.ok(!projection.includes("<repository_skills>"));
  assert.equal(skillsSection([], "F"), undefined);
});

test("skill commands never shadow agy's own commands", () => {
  const skills = [
    { name: "plan", description: "Mine", path: "/w/plan/SKILL.md" },
    { name: "deploy", description: "", path: "/w/deploy/SKILL.md" },
  ];
  assert.deepEqual(skillCommands(skills, [{ name: "plan" }]), [
    {
      name: "deploy",
      description: "Repository skill deploy",
      input: { hint: "what to do with this skill (optional)" },
    },
  ]);
});

test("a skill slash command becomes the instruction to read and follow it", () => {
  const skills = [{ name: "deploy", description: "", path: "/w/deploy/SKILL.md" }];
  const image = { type: "image", data: "x", mimeType: "image/png" };
  const expanded = expandSkillCommand(
    [image, { type: "text", text: "/deploy the web release" }],
    skills,
  );
  assert.equal(expanded[0], image);
  assert.match(
    expanded[1].text,
    /^Use the repository skill "deploy": first read \/w\/deploy\/SKILL\.md/,
  );
  assert.match(expanded[1].text, /Request: the web release$/);
  assert.match(
    expandSkillCommand([{ type: "text", text: "/deploy" }], skills)[0].text,
    /Request: none beyond the skill itself\.$/,
  );
  const plain = [{ type: "text", text: "deploy /deploy" }];
  assert.equal(expandSkillCommand(plain, skills), plain);
  const unknown = [{ type: "text", text: "/other" }];
  assert.equal(expandSkillCommand(unknown, skills), unknown);
  const offered = [{ type: "text", text: "/deploy now" }];
  assert.equal(expandSkillCommand(offered, skills, ["deploy"]), offered, "agy's command wins");
});
