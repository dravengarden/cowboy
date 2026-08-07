// Canonical syntax-language routing for every file-shaped surface (tool reads,
// edits, MCP payloads, and fenced transcript code). The renderer uses
// PrismAsyncLight, so a canonical id loads only when it first appears. Adding a
// file type therefore means one entry here; unknown names deliberately return
// "" and stay readable plaintext.

// This is a data snapshot of Zed 1.13.0's built-in `first_line_pattern`
// matchers. Keep the revision tied to zed-adapter/src/main.rs and update this
// table whenever the pinned Zed revision changes. The entries are in Zed's
// built-in registration order; Zed evaluates the registry in reverse order,
// so languageFromFirstLine does the same below.
export const ZED_LANGUAGE_DETECTION_REVISION =
  "aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45";

const ZED_FIRST_LINE_MAX_COLUMNS = 256;
const ZED_FIRST_LINE_MATCHERS: readonly {
  language: string;
  pattern: RegExp;
}[] = [
  {
    language: "bash",
    pattern: /^#!.*\b(?:ash|bash|bats|dash|sh|zsh)\b/u,
  },
  {
    language: "cpp",
    pattern: /^\/\/.*-\*-\s*C\+\+\s*-\*-/u,
  },
  {
    language: "go",
    pattern: /^\/\/.*\bgo run\b/u,
  },
  {
    language: "python",
    pattern: /^#!.*((\bpython[0-9.]*\b)|(\buv run\b))/u,
  },
  {
    language: "typescript",
    pattern: /^#!.*\b(?:deno run|ts-node|bun|tsx|[/ ]node)\b/u,
  },
  {
    language: "javascript",
    pattern: /^#!.*\b(?:[/ ]node|deno run.*--ext[= ]js)\b/u,
  },
];

const LANGUAGE_ALIASES: Record<string, string> = {
  bash: "bash",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  js: "javascript",
  javascript: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  ts: "typescript",
  typescript: "typescript",
  py: "python",
  python: "python",
  rs: "rust",
  rust: "rust",
  md: "markdown",
  markdown: "markdown",
  yml: "yaml",
  yaml: "yaml",
  html: "markup",
  htm: "markup",
  xml: "markup",
  svg: "markup",
  shellsession: "shell-session",
  console: "shell-session",
  ps1: "powershell",
  cs: "csharp",
  csharp: "csharp",
  cxx: "cpp",
  cc: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  rb: "ruby",
  kt: "kotlin",
  kts: "kotlin",
  objc: "objectivec",
  mm: "objectivec",
  tf: "hcl",
  tfvars: "hcl",
  proto: "protobuf",
  dockerfile: "docker",
  make: "makefile",
  mk: "makefile",
  plaintext: "text",
  txt: "text",
  jq: "jq",
  awk: "awk",
  sed: "bash",
  regex: "regex",
};

const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  ts: "typescript",
  tsx: "tsx",
  js: "javascript",
  jsx: "jsx",
  mjs: "javascript",
  cjs: "javascript",
  json: "json",
  json5: "json5",
  jsonl: "json",
  py: "python",
  pyw: "python",
  rs: "rust",
  go: "go",
  java: "java",
  kt: "kotlin",
  kts: "kotlin",
  swift: "swift",
  c: "c",
  h: "c",
  cc: "cpp",
  cpp: "cpp",
  cxx: "cpp",
  hpp: "cpp",
  hxx: "cpp",
  cs: "csharp",
  m: "objectivec",
  mm: "objectivec",
  rb: "ruby",
  php: "php",
  lua: "lua",
  pl: "perl",
  r: "r",
  scala: "scala",
  dart: "dart",
  ex: "elixir",
  exs: "elixir",
  erl: "erlang",
  fs: "fsharp",
  fsx: "fsharp",
  hs: "haskell",
  ml: "ocaml",
  clj: "clojure",
  cljs: "clojure",
  zig: "zig",
  v: "v",
  sql: "sql",
  gql: "graphql",
  graphql: "graphql",
  css: "css",
  scss: "scss",
  sass: "sass",
  less: "less",
  html: "markup",
  htm: "markup",
  xml: "markup",
  svg: "markup",
  vue: "markup",
  svelte: "markup",
  toml: "toml",
  yaml: "yaml",
  yml: "yaml",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  properties: "properties",
  env: "bash",
  md: "markdown",
  markdown: "markdown",
  rst: "rest",
  adoc: "asciidoc",
  tex: "latex",
  sh: "bash",
  shell: "bash",
  bash: "bash",
  zsh: "bash",
  fish: "bash",
  ps1: "powershell",
  bat: "batch",
  cmd: "batch",
  nix: "nix",
  cue: "cue",
  hcl: "hcl",
  tf: "hcl",
  tfvars: "hcl",
  proto: "protobuf",
  dockerfile: "docker",
  mk: "makefile",
  cmake: "cmake",
  gradle: "gradle",
  groovy: "groovy",
  wasm: "wasm",
  wat: "wasm",
  diff: "diff",
  patch: "diff",
  csv: "csv",
  tsv: "csv",
  jq: "jq",
  nginx: "nginx",
  service: "systemd",
  socket: "systemd",
  timer: "systemd",
  target: "systemd",
};

const LANGUAGE_BY_BASENAME: Record<string, string> = {
  dockerfile: "docker",
  containerfile: "docker",
  makefile: "makefile",
  gnumakefile: "makefile",
  "cmakelists.txt": "cmake",
  "go.mod": "go-module",
  "go.sum": "go-module",
  "cargo.toml": "toml",
  "cargo.lock": "toml",
  "flake.nix": "nix",
  justfile: "makefile",
  procfile: "bash",
  gemfile: "ruby",
  rakefile: "ruby",
};

export function normalizeSyntaxLanguage(hint: string): string {
  const normalized = hint.trim().toLowerCase().replace(/^language-/u, "");
  return LANGUAGE_ALIASES[normalized] ?? normalized;
}

export function languageFromFirstLine(content: string): string {
  const firstLine = content.split(/\r\n?|\n/u, 1)[0]?.slice(
    0,
    ZED_FIRST_LINE_MAX_COLUMNS,
  ) ?? "";
  for (let index = ZED_FIRST_LINE_MATCHERS.length - 1; index >= 0; index--) {
    const matcher = ZED_FIRST_LINE_MATCHERS[index];
    if (matcher?.pattern.test(firstLine)) return matcher.language;
  }
  return "";
}

export function languageFromPath(path: string): string {
  const clean = path.split(/[?#]/u, 1)[0]?.replaceAll("\\", "/") ?? "";
  const basename = clean.split("/").pop()?.toLowerCase() ?? "";
  if (!basename) return "";
  const exact = LANGUAGE_BY_BASENAME[basename];
  if (exact) return exact;
  if (basename.startsWith("dockerfile.")) return "docker";
  if (/^\.env(?:\..+)?$/u.test(basename)) return "bash";
  const extension = basename.includes(".")
    ? basename.split(".").pop() ?? ""
    : "";
  return LANGUAGE_BY_EXTENSION[extension] ?? "";
}
