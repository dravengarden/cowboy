import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  DEFAULT_VIM_TIMEOUT_MS,
  escapeMappingLine,
  InsertEscapeMatcher,
  insertEscapeSequences,
  parseVimConfig,
  selectedEscapePreset,
  withEscapePreset,
} from "./vimConfig";

test("escape presets replace one line and leave the rest of the config", () => {
  const source = "nnoremap H ^\ninoremap jk <Esc>";
  assertEquals(selectedEscapePreset(source), "jk");
  const next = withEscapePreset(source, "jj");
  assertEquals(next, "inoremap jj <Esc>\nnnoremap H ^");
  assertEquals(selectedEscapePreset(next), "jj");
  assertEquals(withEscapePreset(next, null), "nnoremap H ^");
});

test("a vimrc subset parses mappings, timeoutlen and comments", () => {
  const config = parseVimConfig([
    '" leave insert without reaching for Esc',
    "inoremap jj <Esc>",
    ":nnoremap <silent> H ^",
    "map Y y$",
    "set timeoutlen=600",
    "",
  ].join("\n"));
  assertEquals(config.errors, []);
  assertEquals(config.timeoutMs, 600);
  assertEquals(config.mappings, [
    { lhs: "jj", rhs: "<Esc>", mode: "insert", recursive: false },
    { lhs: "H", rhs: "^", mode: "normal", recursive: false },
    { lhs: "Y", rhs: "y$", mode: undefined, recursive: true },
  ]);
});

test("unsupported lines are reported with their line number and skipped", () => {
  const config = parseVimConfig("set number\nfoo bar\nnmap x\nset timeoutlen=10\nimap jk <Esc>");
  assertEquals(config.errors.map((error) => error.line), [1, 2, 3, 4]);
  assertEquals(config.timeoutMs, DEFAULT_VIM_TIMEOUT_MS);
  assertEquals(config.mappings.length, 1);
});

test("only typed insert sequences that map to Esc reach native fields", () => {
  const config = parseVimConfig([
    escapeMappingLine("jj"),
    "imap kj <C-[>",
    "inoremap <C-c> <Esc>",
    "nnoremap jj <Esc>",
    "inoremap jw <C-w>",
    "map j <Esc>",
  ].join("\n"));
  assertEquals(insertEscapeSequences(config), ["jj", "kj"]);
});

test("the matcher completes a sequence only within the timeout", () => {
  let now = 0;
  const matcher = new InsertEscapeMatcher(() => now);
  const sequences = ["jj"];
  assertEquals(matcher.push("j", sequences, 1000), null);
  now = 300;
  assertEquals(matcher.push("j", sequences, 1000), "jj");
  // A slow second j is two typed letters, not an escape.
  assertEquals(matcher.push("j", sequences, 1000), null);
  now = 1500;
  assertEquals(matcher.push("j", sequences, 1000), null);
  now = 1600;
  assertEquals(matcher.push("x", sequences, 1000), null);
  assertEquals(matcher.push("j", sequences, 1000), null);
  assertEquals(matcher.push("j", sequences, 1000), "jj");
});

test("overlapping sequences keep the longest live prefix", () => {
  const matcher = new InsertEscapeMatcher(() => 0);
  const sequences = ["jk", "kj"];
  assertEquals(matcher.push("j", sequences, 1000), null);
  assertEquals(matcher.push("j", sequences, 1000), null);
  assertEquals(matcher.push("k", sequences, 1000), "jk");
  assertEquals(matcher.push("k", sequences, 1000), null);
  assertEquals(matcher.push("j", sequences, 1000), "kj");
});
