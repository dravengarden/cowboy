import { assertEquals } from "jsr:@std/assert";
import { languageFromPath, normalizeSyntaxLanguage } from "./syntaxLanguages";

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
  assertEquals(languageFromPath("/repo/LICENSE"), "");
  assertEquals(languageFromPath("/repo/archive.unknown"), "");
});
