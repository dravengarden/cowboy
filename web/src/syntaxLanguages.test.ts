import { assertEquals } from "jsr:@std/assert";
import {
  languageFromFirstLine,
  languageFromPath,
  normalizeSyntaxLanguage,
} from "./syntaxLanguages";

Deno.test("file language routing covers mainstream and extensionless files", () => {
  assertEquals(languageFromPath("/repo/src/main.go"), "go");
  assertEquals(languageFromPath("C:\\repo\\Program.cs"), "csharp");
  assertEquals(languageFromPath("/repo/Dockerfile.dev"), "docker");
  assertEquals(languageFromPath("/repo/CMakeLists.txt"), "cmake");
  assertEquals(languageFromPath("/repo/.env.production"), "bash");
  assertEquals(languageFromPath("/repo/module.tf?raw=1"), "hcl");
});

Deno.test("language routing normalizes aliases and safely leaves unknown files plain", () => {
  assertEquals(normalizeSyntaxLanguage("language-js"), "javascript");
  assertEquals(normalizeSyntaxLanguage("jq"), "jq");
  assertEquals(normalizeSyntaxLanguage("AWK"), "awk");
  assertEquals(normalizeSyntaxLanguage("regex"), "regex");
  assertEquals(normalizeSyntaxLanguage("sed"), "bash");
  assertEquals(normalizeSyntaxLanguage("SH"), "bash");
  assertEquals(normalizeSyntaxLanguage("shell"), "bash");
  assertEquals(languageFromPath("/repo/LICENSE"), "");
  assertEquals(languageFromPath("/repo/archive.unknown"), "");
});

Deno.test("Zed first-line routing recognizes its built-in matchers", () => {
  assertEquals(
    languageFromFirstLine("#!/usr/bin/env bash\nprintf '%s\\n' ok\n"),
    "bash",
  );
  assertEquals(languageFromFirstLine("#!/usr/bin/env python3\n"), "python");
  assertEquals(languageFromFirstLine("//go:build linux // go run\n"), "go");
  assertEquals(languageFromFirstLine("#!/usr/bin/env node\n"), "javascript");
  assertEquals(
    languageFromFirstLine("#!/usr/bin/env deno run --ext=ts\n"),
    "typescript",
  );
  assertEquals(languageFromFirstLine("#!/usr/bin/env perl\n"), "");
});
