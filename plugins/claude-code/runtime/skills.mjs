// Target skills and custom commands for a bound session. Native Claude Code
// 2.1.287 discovers them locally from (measured):
//
// - ~/.claude/skills/<name>/SKILL.md and ~/.claude/commands/**/*.md;
// - .claude/skills and .claude/commands of each directory from the working
//   directory up to the repository root, never the home directory itself.
//
// A user skill shadows a project skill of the same name, a skill a command.
// Natively these are local files. Here the target's files are copied into a
// private plugin native loads; context-mod.js shows them under their native
// names and paths and runs their inline shell commands on the target.
import { mkdir, writeFile } from "node:fs/promises";
import { join, posix } from "node:path";
import { withoutComment } from "./instructions.mjs";

// Native namespaces plugin skills by plugin name; context-mod.js removes it.
export const SKILL_PLUGIN = "cowboy-target";
const MAX_SKILL_FILE = 1024 * 1024;
const MAX_SKILLS = 512;
const MAX_SKILL_BYTES = 16 * 1024 * 1024;

// Bundled skills whose instructions work through the target-bound tools.
export const BUNDLED_SKILLS = [
  "code-review",
  "init",
  "keybindings-help",
  "run",
  "security-review",
  "simplify",
  "update-config",
];
// Bundled skills native lists that cannot work against the target.
export const UNAVAILABLE_BUNDLED = {
  "claude-api":
    "its reference files are on the machine running this session, not on the target",
  dataviz:
    "its reference files and validators are on the machine running this session, not on the target",
  "plugin-authoring":
    "its reference files are on the machine running this session, not on the target",
  "fewer-permission-prompts":
    "it scans session transcripts, which are kept on the machine running this session",
  loop:
    "it schedules prompts with the Cron tools, which this session does not have",
  "workflow-authoring":
    "it writes scripts for the Workflow tool, which this session does not have",
};

// The directories native would search, in its order of precedence.
export function skillRoots(cwd, home, repositoryRoot) {
  const projects = [];
  for (let directory = cwd;; directory = posix.dirname(directory)) {
    if (directory === home) break;
    projects.push(directory);
    if (directory === repositoryRoot || directory === "/") break;
  }
  const at = (scope, kind, directory) => ({ scope, kind, directory });
  return [
    ...(home
      ? [at("user", "skill", posix.join(home, ".claude", "skills"))]
      : []),
    ...projects.map((directory) =>
      at("project", "skill", posix.join(directory, ".claude", "skills"))
    ),
    ...(home
      ? [at("user", "command", posix.join(home, ".claude", "commands"))]
      : []),
    ...projects.map((directory) =>
      at("project", "command", posix.join(directory, ".claude", "commands"))
    ),
  ];
}

// The native name of a listed file under its root, or undefined.
export function skillName(root, path) {
  if (!path.startsWith(root.directory + "/")) return undefined;
  const parts = path.slice(root.directory.length + 1).split("/");
  // A command's name joins its path with colons, so its parts have none.
  if (
    parts.some((part) =>
      !part || part.startsWith(".") || /[\u0000-\u001f\u007f]/.test(part) ||
      (root.kind === "command" && part.includes(":"))
    )
  ) return undefined;
  if (root.kind === "skill") {
    return parts.length === 2 && parts[1] === "SKILL.md" ? parts[0] : undefined;
  }
  if (!parts.at(-1).endsWith(".md") || parts.at(-1) === ".md") return undefined;
  return [...parts.slice(0, -1), parts.at(-1).slice(0, -3)].join(":");
}

// The frontmatter fields that decide how a target skill can be offered.
// Its hooks would run where this session runs; so might those of a
// frontmatter whose top-level keys are not all plain (quoted or escaped keys,
// merges, complex keys or one flow mapping), which this reading cannot rule
// out. Top-level keys may share an indentation.
export function skillFrontmatter(text) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---(?:\r?\n|$)/.exec(text);
  const fields = {
    hooks: false,
    shell: undefined,
    allowedTools: [],
    description: undefined,
    indent: "",
  };
  if (!match) return fields;
  const lines = match[1].split(/\r?\n/).map(withoutComment);
  fields.indent = /^\s*/.exec(lines.find((line) => line.trim()) ?? "")[0];
  let list = null;
  // A list may follow its key without indentation.
  let listed = false;
  for (const indented of lines) {
    if (!indented.trim()) continue;
    if (!indented.startsWith(fields.indent)) {
      fields.hooks = true;
      continue;
    }
    const line = indented.slice(fields.indent.length);
    const unindentedItem = listed && /^-(\s|$)/.test(line);
    if (
      /^\S/.test(line) && !unindentedItem &&
      !/^[A-Za-z_][\w-]*\s*:(\s|$)/.test(line)
    ) fields.hooks = true;
    const key = /^([A-Za-z_][\w-]*)\s*:\s*(.*?)\s*$/.exec(line);
    if (key) {
      list = null;
      listed = key[2] === "";
      const value = key[2].replace(/^(["'])(.*)\1$/, "$2");
      if (key[1].toLowerCase().replace(/[-_]/g, "") === "hooks") {
        fields.hooks = true;
      }
      if (key[1] === "shell") fields.shell = value;
      if (key[1] === "description") fields.description = value;
      if (key[1] === "allowed-tools") {
        if (value) fields.allowedTools.push(...toolRules(value));
        else list = fields.allowedTools;
      }
      continue;
    }
    const item = /^\s*-\s*(.+?)\s*$/.exec(line);
    if (list && item) list.push(item[1].replace(/^(["'])(.*)\1$/, "$2"));
  }
  return fields;
}

// Rules of an allowed-tools string or flow list: commas or spaces outside
// parentheses separate them.
function toolRules(value) {
  const body = value.startsWith("[") && value.endsWith("]")
    ? value.slice(1, -1)
    : value;
  const rules = [];
  let current = "";
  let depth = 0;
  for (const char of body) {
    if (char === "(") depth++;
    if (char === ")") depth--;
    if (depth === 0 && (char === "," || /\s/.test(char))) {
      if (current) rules.push(current);
      current = "";
    } else current += char;
  }
  if (current) rules.push(current);
  return rules.map((rule) => rule.replace(/^(["'])(.*)\1$/, "$2"));
}

// The skills and commands offered, in native precedence, with those that
// cannot run here and why.
export function targetSkills(found) {
  const entries = [];
  const omitted = [];
  const names = new Set();
  let total = 0;
  for (const file of found) {
    if (names.has(file.name)) continue;
    names.add(file.name);
    const fields = skillFrontmatter(file.content);
    const reason = fields.hooks
      ? "it may declare hooks, which would run on the machine running this session"
      : fields.shell !== undefined && fields.shell !== "bash"
      ? `it runs its commands in ${fields.shell}`
      : Buffer.byteLength(file.content) > MAX_SKILL_FILE
      ? "its file exceeds 1 MiB"
      : entries.length >= MAX_SKILLS ||
          total + Buffer.byteLength(file.content) > MAX_SKILL_BYTES
      ? "the session's skill limit was reached"
      : undefined;
    if (reason) {
      omitted.push({ name: file.name, reason });
      continue;
    }
    total += Buffer.byteLength(file.content);
    entries.push({
      name: file.name,
      kind: file.kind,
      scope: file.scope,
      path: file.path,
      content: file.content,
      allowedTools: fields.allowedTools,
    });
  }
  return { entries, omitted };
}

// Positions of inline `!` commands native would run: a `!` at a line start
// or after whitespace, then a backtick span. Other code spans are masked
// first, as natively.
function inlineCommands(text) {
  const masked = text.replace(/`[^`\n]+`/g, (span, at) => {
    const before = text[at - 1];
    return before === "!" || before === "`"
      ? span
      : "`" + " ".repeat(span.length - 2) + "`";
  });
  return [...masked.matchAll(/(?<=^|\s)!`[^`]+`/gm)].map((match) =>
    match.index
  );
}

// The description native gives a local skill or command without one: the
// first non-empty line, without heading marks, cut at 100 characters
// (measured on 2.1.287). A plugin's is not listed without one.
export function derivedDescription(body, kind) {
  const line = body.split(/\r?\n/).map((item) => item.trim()).find(Boolean)
    ?.replace(/^#+\s*/, "");
  if (!line) return kind === "skill" ? "Skill" : "Custom command";
  return line.length > 100 ? line.slice(0, 97) + "..." : line;
}

// A target skill's file as native loads it from the private plugin. Its
// directory placeholders name the target. Its shell commands carry this
// session's marker, so native leaves them for context-mod.js to run on the
// target after argument substitution.
export function mirrorContent(entry, marker, cwd) {
  let text = entry.content;
  if (entry.kind === "skill") {
    const directory = posix.dirname(entry.path);
    text = text.replaceAll("${CLAUDE_SKILL_DIR}", () => directory);
  }
  const original = /^---\r?\n[\s\S]*?\r?\n---(?:\r?\n|$)/.exec(text)?.[0] ?? "";
  let body = text.slice(original.length).replaceAll(
    "${CLAUDE_PROJECT_DIR}",
    () => cwd,
  );
  // Native lists a plugin's skill only with a description: give it the one
  // native would derive for the local file.
  const fields = skillFrontmatter(text);
  const description = fields.indent + "description: " +
    JSON.stringify(derivedDescription(body, entry.kind));
  const front = !original
    ? `---\n${description}\n---\n`
    : fields.description === ""
    ? original.replace(
      /^[ \t]*description:[^\n]*$/m,
      () => description,
    )
    : fields.description !== undefined
    ? original
    : original.replace(/\r?\n---(\r?\n|$)$/, (end) => `\n${description}${end}`);
  const inserts = [
    ...[...body.matchAll(/```!/g)].map((match) => match.index + 3),
    ...inlineCommands(body).map((at) => at + 1),
  ].sort((left, right) => right - left);
  for (const at of inserts) body = body.slice(0, at) + marker + body.slice(at);
  return front + body;
}

// The private plugin directory and what context-mod.js needs to present it.
export async function writeSkillPlugin(root, skills, marker, cwd) {
  await mkdir(join(root, ".claude-plugin"), { recursive: true });
  await writeFile(
    join(root, ".claude-plugin", "plugin.json"),
    JSON.stringify({ name: SKILL_PLUGIN, version: "1.0.0" }),
    { flag: "wx" },
  );
  const entries = [];
  for (const entry of skills.entries) {
    const parts = entry.kind === "skill"
      ? ["skills", entry.name, "SKILL.md"]
      : [
        "commands",
        ...entry.name.split(":").slice(0, -1),
        entry.name.split(":").at(-1) + ".md",
      ];
    const file = join(root, ...parts);
    if (
      !file.startsWith(root + "/") ||
      parts.some((part) => !part || part === "." || part === "..")
    ) throw new Error("Invalid target skill name");
    await mkdir(join(file, ".."), { recursive: true });
    const content = mirrorContent(entry, marker, cwd);
    await writeFile(file, content, { flag: "wx" });
    entries.push({
      name: entry.name,
      kind: entry.kind,
      scope: entry.scope,
      ...(entry.kind === "skill"
        ? {
          mirrorDirectory: join(root, "skills", entry.name),
          targetDirectory: posix.dirname(entry.path),
        }
        : {}),
      // As native reads them: with the skill's directory placeholder named.
      allowedTools: skillFrontmatter(content).allowedTools,
    });
  }
  return {
    prefix: SKILL_PLUGIN + ":",
    marker,
    entries,
    omitted: skills.omitted,
    bundled: BUNDLED_SKILLS,
    unavailable: UNAVAILABLE_BUNDLED,
  };
}
