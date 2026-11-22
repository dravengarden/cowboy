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

async function fixture(t, options = {}) {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-skills-")));
  const root = join(base, "workspace");
  mkdirSync(join(root, "sub"), { recursive: true });
  const socket = join(base, "exec.sock");
  const endpoint = await fakeEndpoint(socket, root, options);
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
  return { base, root, facade, endpoint };
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
  assert.match(projection, /<skills>/);
  assert.match(projection, /- deploy: Sub deploy \(/);
  assert.ok(!projection.includes("SECRET_BODY"), "only the frontmatter is projected");
  const fence = /(COWBOY_UNTRUSTED_[a-f0-9]{32})/.exec(projection)[1];
  const listed = projection.slice(projection.indexOf("<skills>"));
  // The intro names the fence once; the list sits between two more markers.
  assert.equal(listed.split(fence).length, 4, "the list is fenced");
});

test("the user's home skills are listed through their symlinks; the repository's win", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-skills-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const home = join(base, "home");
  const store = join(base, "store");
  // ~/.agents/skills is a symlinked directory; ~/.claude/skills a real one
  // whose entry is a symlinked skill directory.
  skill(join(store, "skills"), "shared", "---\nname: shared\ndescription: Shared\n---");
  skill(join(store, "skills"), "deploy", "---\nname: deploy\ndescription: Home deploy\n---");
  skill(store, "linked", "---\nname: linked\ndescription: Linked\n---");
  mkdirSync(join(home, ".agents"), { recursive: true });
  symlinkSync(join(store, "skills"), join(home, ".agents", "skills"));
  mkdirSync(join(home, ".claude", "skills"), { recursive: true });
  symlinkSync(join(store, "linked"), join(home, ".claude", "skills", "linked"));
  const { root, facade } = await fixture(t, { home });
  skill(join(root, ".agents", "skills"), "deploy", "---\nname: deploy\ndescription: Repo\n---");
  let reported;
  const projection = await buildProjection(facade, root, {
    onSkills: (value) => {
      reported = value;
    },
  });
  assert.deepEqual(reported, [
    {
      name: "deploy",
      description: "Repo",
      path: join(root, ".agents", "skills", "deploy", "SKILL.md"),
    },
    {
      name: "linked",
      description: "Linked",
      path: join(home, ".claude", "skills", "linked", "SKILL.md"),
      user: true,
    },
    {
      name: "shared",
      description: "Shared",
      path: join(home, ".agents", "skills", "shared", "SKILL.md"),
      user: true,
    },
  ]);
  assert.match(projection, /<skills>\nThese skills are available/);
  assert.ok(!projection.includes(store), "user skills keep their home paths");
  assert.match(
    expandSkillCommand([{ type: "text", text: "/shared" }], reported)[0].text,
    /^Use the user skill "shared"/,
  );
});

test("repository skills are kept first when the list is cut", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-skills-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const home = join(base, "home");
  for (let index = 0; index < SKILL_LIMITS.skills + 6; index++) {
    const name = `h${String(index).padStart(2, "0")}`;
    skill(join(home, ".codex", "skills"), name, `---\nname: ${name}\n---`);
  }
  const { root, facade } = await fixture(t, { home });
  skill(join(root, ".claude", "skills"), "zz-repo", "---\nname: zz-repo\n---");
  const skills = await discoverSkills(facade, [root], root, home);
  assert.equal(skills.length, SKILL_LIMITS.skills);
  assert.equal(skills.at(-1).name, "zz-repo");
  assert.equal(skills.filter((entry) => entry.user).length, SKILL_LIMITS.skills - 1);
});

test("a home inside the repository follows the repository's rules", async (t) => {
  const { base, root, facade } = await fixture(t);
  mkdirSync(join(base, "outside", "evil"), { recursive: true });
  writeFileSync(join(base, "outside", "evil", "SKILL.md"), "---\nname: evil\n---\n");
  mkdirSync(join(root, ".codex", "skills"), { recursive: true });
  symlinkSync(join(base, "outside", "evil"), join(root, ".codex", "skills", "evil"));
  skill(join(root, ".codex", "skills"), "own", "---\nname: own\n---");
  const skills = await discoverSkills(facade, [root], root, root);
  assert.deepEqual(skills, [
    { name: "own", description: "", path: join(root, ".codex", "skills", "own", "SKILL.md") },
  ]);
});

test("the scan stops once enough skills are found, repository first", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-skills-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const home = join(base, "home");
  for (let index = 0; index < 200; index++) {
    const name = `h${String(index).padStart(3, "0")}`;
    skill(join(home, ".claude", "skills"), name, `---\nname: ${name}\n---`);
  }
  const { root, facade, endpoint } = await fixture(t, { home });
  skill(join(root, ".agents", "skills"), "zz-repo", "---\nname: zz-repo\n---");
  const skills = await discoverSkills(facade, [root], root, home);
  assert.equal(skills.length, SKILL_LIMITS.skills);
  assert.ok(skills.some((entry) => entry.name === "zz-repo"));
  const reads = endpoint.calls.filter((entry) => entry.method === "fs/readFile").length;
  assert.ok(reads <= SKILL_LIMITS.skills + SKILL_LIMITS.concurrency, `${reads} reads`);
});

test("repository skills are listed first under the section bound, shown by name", () => {
  const description = "d".repeat(500);
  const skills = [
    ...Array.from({ length: 60 }, (_, index) => ({
      name: `u${String(index).padStart(2, "0")}`,
      description,
      path: `/h/${index}/SKILL.md`,
      user: true,
    })),
    { name: "zz-repo", description: "Repo", path: "/w/zz/SKILL.md" },
    { name: "aa-repo", description: "Repo", path: "/w/aa/SKILL.md" },
  ];
  const section = skillsSection(skills, `COWBOY_UNTRUSTED_${"f".repeat(32)}`);
  assert.ok(Buffer.byteLength(section) <= SKILL_LIMITS.section, `${Buffer.byteLength(section)}`);
  assert.match(section, /- zz-repo: Repo/);
  assert.match(section, /\[truncated: skill list limit reached\]/);
  const names = [...section.matchAll(/^- (\S+):/gm)].map((match) => match[1]);
  assert.deepEqual(names, [...names].sort());
  assert.equal(names[0], "aa-repo");
});

test("a repository that supplies enough skills never waits for the home", async (t) => {
  const base = realpathSync(mkdtempSync(join(tmpdir(), "agy-home-skills-")));
  t.after(() => rmSync(base, { recursive: true, force: true }));
  const home = join(base, "home");
  skill(join(home, ".claude", "skills"), "slow", "---\nname: slow\n---");
  // A home mount that answers only after 30 s.
  const { root, facade, endpoint } = await fixture(t, {
    home,
    delay: (method, params) => (String(params?.path ?? "").includes(home) ? 30_000 : 0),
  });
  for (let index = 0; index < SKILL_LIMITS.skills; index++) {
    const name = `r${String(index).padStart(2, "0")}`;
    skill(join(root, ".claude", "skills"), name, `---\nname: ${name}\n---`);
  }
  const started = Date.now();
  const skills = await discoverSkills(facade, [root], root, home);
  assert.ok(Date.now() - started < 10_000, `${Date.now() - started} ms`);
  assert.equal(skills.length, SKILL_LIMITS.skills);
  assert.ok(!skills.some((entry) => entry.user));
  assert.ok(!endpoint.calls.some((entry) => String(entry.params?.path ?? "").includes(home)));
});

test("a home that resolves into the repository gets no symlink handling", async (t) => {
  const { base, root, facade } = await fixture(t);
  mkdirSync(join(base, "outside", "evil"), { recursive: true });
  writeFileSync(join(base, "outside", "evil", "SKILL.md"), "---\nname: evil\n---\n");
  mkdirSync(join(root, "sub", ".claude", "skills"), { recursive: true });
  symlinkSync(join(base, "outside", "evil"), join(root, "sub", ".claude", "skills", "evil"));
  symlinkSync(join(root, "sub"), join(base, "linked-home"));
  const skills = await discoverSkills(facade, [root], root, join(base, "linked-home"));
  assert.deepEqual(skills, []);
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
  assert.ok(!projection.includes("<skills>"));
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

test("a managed projection lists no repository skills", async (t) => {
  const { root, facade } = await fixture(t);
  skill(join(root, ".agents", "skills"), "deploy", "---\nname: deploy\ndescription: Ship\n---");
  let reported;
  const projection = await buildProjection(facade, root, {
    routing: "MANAGED ROUTING",
    skills: false,
    onSkills: (value) => {
      reported = value;
    },
  });
  assert.deepEqual(reported, []);
  assert.match(projection, /<cowboy_routing>\nMANAGED ROUTING\n<\/cowboy_routing>/);
  assert.ok(!projection.includes("<skills>"));
  assert.ok(!projection.includes("deploy"));
});
