// Project instruction files on the target, discovered as native Claude Code
// 2.1.287 discovers them locally (measured; AGENTS.md is not one of them):
//
// - the user's ~/.claude/CLAUDE.md and unconditional ~/.claude/rules/*.md;
// - for each directory from the root down to the working directory, its
//   CLAUDE.md, .claude/CLAUDE.md, unconditional .claude/rules/**/*.md and
//   CLAUDE.local.md;
// - `@path` imports of any of them, up to five levels, after their importer;
// - on a Read below the working directory, the CLAUDE.md files of the
//   directories in between and the rules whose `paths` match, each once.
//
// Native renders the files itself through `prompt.context`'s list.
import { posix } from "node:path";

const MAX_IMPORT_DEPTH = 5;

// Frontmatter of a rule file: its `paths` globs, when it is conditional.
export function ruleScope(text) {
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (!match) return { content: text, paths: [] };
  const paths = [];
  let inPaths = false;
  let flow = null;
  for (const raw of match[1].split(/\r?\n/)) {
    const line = withoutComment(raw);
    if (flow !== null) {
      // A flow sequence may span lines until its bracket closes.
      flow += " " + line.trim();
      if (flowClosed(flow)) {
        paths.push(...flowItems(flow));
        flow = null;
      }
      continue;
    }
    const key = /^([A-Za-z_]+):\s*(.*)$/.exec(line);
    if (key) {
      inPaths = key[1] === "paths";
      const value = key[2].trim();
      if (inPaths && value.startsWith("[") && !flowClosed(value)) {
        flow = value;
      } else if (inPaths && value) paths.push(...flowItems(value));
      continue;
    }
    const item = /^\s*-\s*(.+)$/.exec(line);
    if (inPaths && item) paths.push(...flowItems(item[1].trim()));
  }
  return { content: text.slice(match[0].length), paths };
}

// Whether a flow sequence's own closing bracket (outside quotes) is present.
function flowClosed(text) {
  let quote = null;
  let depth = 0;
  for (const char of text) {
    if (quote) {
      if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "[") depth++;
    else if (char === "]" && --depth === 0) return true;
  }
  return false;
}

// A YAML line without its trailing ` # comment` (outside quotes).
function withoutComment(line) {
  let quote = null;
  for (let index = 0; index < line.length; index++) {
    const char = line[index];
    if (quote) {
      if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "#" && (index === 0 || /\s/.test(line[index - 1]))) {
      return line.slice(0, index).trimEnd();
    }
  }
  return line;
}

// Items of a YAML flow sequence or scalar, keeping commas inside quotes and
// braces (`["src/**/*.{ts,tsx}", 'a,b']`).
function flowItems(value) {
  const body = value.startsWith("[") && value.endsWith("]")
    ? value.slice(1, -1)
    : value;
  const items = [];
  let current = "";
  let quote = null;
  let depth = 0;
  for (const char of body) {
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "{") (depth++, current += char);
    else if (char === "}") (depth--, current += char);
    else if (char === "," && depth === 0) {
      (items.push(current.trim()), current = "");
    } else current += char;
  }
  items.push(current.trim());
  return items.filter(Boolean);
}

// `{a,b}` alternatives expanded into separate globs.
function expandBraces(glob) {
  const open = glob.indexOf("{");
  if (open < 0) return [glob];
  let depth = 0;
  const parts = [];
  let start = open + 1;
  for (let index = open; index < glob.length; index++) {
    if (glob[index] === "{") depth++;
    else if (glob[index] === "}" && --depth === 0) {
      parts.push(glob.slice(start, index));
      const tail = glob.slice(index + 1);
      return parts.flatMap((part) =>
        expandBraces(glob.slice(0, open) + part + tail)
      );
    } else if (glob[index] === "," && depth === 1) {
      parts.push(glob.slice(start, index));
      start = index + 1;
    }
  }
  return [glob];
}

export function globMatches(glob, path) {
  return expandBraces(glob).some((single) => singleGlobMatches(single, path));
}

function singleGlobMatches(glob, path) {
  let pattern = "";
  for (let index = 0; index < glob.length; index++) {
    const char = glob[index];
    if (char === "*" && glob[index + 1] === "*") {
      index++;
      if (glob[index + 1] === "/") {
        index++;
        pattern += "(?:.*/)?";
      } else pattern += ".*";
    } else if (char === "*") pattern += "[^/]*";
    else if (char === "?") pattern += "[^/]";
    else if (char === "[") {
      const close = glob.indexOf("]", index + 1);
      if (close < 0) pattern += "\\[";
      else {
        const set = glob.slice(index + 1, close).replace(/^!/, "^")
          .replace(/\\/g, "\\\\");
        pattern += `[${set}]`;
        index = close;
      }
    } else pattern += char.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`^${pattern}$`).test(path);
}

// `@path` references outside code, as native resolves them.
export function importsOf(text, file, home) {
  const imports = [];
  const prose = text.replace(/```[\s\S]*?```/g, "").replace(/`[^`\n]*`/g, "");
  for (
    const match of prose.matchAll(/(?:^|\s)@((?:~\/|\.{0,2}\/|[\w.-])[^\s]*)/g)
  ) {
    let target = match[1];
    if (target.startsWith("~/")) {
      if (!home) continue;
      target = posix.join(home, target.slice(2));
    } else if (!target.startsWith("/")) {
      target = posix.join(posix.dirname(file), target);
    }
    imports.push(posix.normalize(target));
  }
  return imports;
}

// `read(path)` resolves a file's text, or undefined when absent or unreadable;
// `rules(directory)` the sorted rule files under it.
export async function instructionFiles({ cwd, home, read, rules }) {
  const files = [];
  const seen = new Set();
  const conditional = [];
  const add = async (path, kind, parent, depth = 0) => {
    if (seen.has(path)) return;
    const text = await read(path);
    if (text === undefined) return;
    seen.add(path);
    files.push({ path, kind, content: text, ...(parent ? { parent } : {}) });
    if (depth >= MAX_IMPORT_DEPTH) return;
    for (const target of importsOf(text, path, home)) {
      await add(target, kind, path, depth + 1);
    }
  };
  // Scoped rules match project-relative paths, the user's own included.
  const addRules = async (directory, kind, root = directory) => {
    for (const path of await rules(posix.join(directory, ".claude", "rules"))) {
      const text = await read(path);
      if (text === undefined) continue;
      const scoped = ruleScope(text);
      if (scoped.paths.length) {
        // Collected once: a home that is also an ancestor keeps the user
        // rule's project-relative matching.
        if (conditional.some((rule) => rule.path === path)) continue;
        conditional.push({
          path,
          root,
          paths: scoped.paths,
          content: scoped.content,
        });
        continue;
      }
      if (seen.has(path)) continue;
      seen.add(path);
      files.push({ path, kind, content: scoped.content });
      // A rule's own imports follow it, as any instruction file's do.
      for (const target of importsOf(scoped.content, path, home)) {
        await add(target, kind, path, 1);
      }
    }
  };
  if (home) {
    await add(posix.join(home, ".claude", "CLAUDE.md"), "user");
    await addRules(home, "user", cwd);
  }
  const directories = [];
  for (let directory = cwd;; directory = posix.dirname(directory)) {
    directories.unshift(directory);
    if (directory === "/") break;
  }
  for (const directory of directories) {
    await add(posix.join(directory, "CLAUDE.md"), "project");
    await add(posix.join(directory, ".claude", "CLAUDE.md"), "project");
    await addRules(directory, "project");
    await add(posix.join(directory, "CLAUDE.local.md"), "local");
  }
  return { files, conditional };
}

// What a Read of `path` attaches natively: CLAUDE.md files of the directories
// between the working directory and the file, then matching scoped rules;
// `attached` holds what this session has already shown.
export async function nestedInstructions(
  { cwd, path, read, conditional, attached, home },
) {
  const relative = posix.relative(cwd, path);
  if (
    !relative || relative.startsWith("../") || relative === ".." ||
    posix.isAbsolute(relative)
  ) return [];
  const found = [];
  // Marked as shown only once everything is read: a failed read leaves the
  // files for a later Read.
  const shown = new Set();
  const seen = (file) => attached.has(file) || shown.has(file);
  // A nested file's own imports follow it, as at session start.
  const attach = async (file, depth) => {
    if (seen(file)) return;
    const text = await read(file);
    if (text === undefined) return;
    shown.add(file);
    found.push({ path: file, content: text });
    if (depth >= MAX_IMPORT_DEPTH) return;
    for (const target of importsOf(text, file, home)) {
      await attach(target, depth + 1);
    }
  };
  const parts = posix.dirname(relative).split("/").filter((part) =>
    part !== "."
  );
  let directory = cwd;
  for (const part of parts) {
    directory = posix.join(directory, part);
    await attach(posix.join(directory, "CLAUDE.md"), 0);
  }
  for (const rule of conditional) {
    if (seen(rule.path)) continue;
    const local = posix.relative(rule.root, path);
    if (
      local.startsWith("../") ||
      !rule.paths.some((glob) => globMatches(glob, local))
    ) continue;
    shown.add(rule.path);
    found.push({ path: rule.path, content: rule.content });
    for (const target of importsOf(rule.content, rule.path, home)) {
      await attach(target, 1);
    }
  }
  for (const file of shown) attached.add(file);
  return found;
}

// Native's text for one nested file.
export function nestedText(file) {
  return `Contents of ${file.path}:\n\n${file.content}`;
}
