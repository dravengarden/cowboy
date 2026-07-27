import { assertEquals } from "jsr:@std/assert";
import { languageDescriptionForPath } from "./codeLanguage.ts";

Deno.test("Code language routing recognizes common repository files", () => {
  assertEquals(languageDescriptionForPath("src/main.rs")?.name, "Rust");
  assertEquals(languageDescriptionForPath("web/src/main.tsx")?.name, "TSX");
  assertEquals(languageDescriptionForPath("Cargo.lock")?.name, "TOML");
  assertEquals(languageDescriptionForPath("docs/guide.md")?.name, "Markdown");
});

Deno.test("Code language routing leaves unknown binary-shaped names plain", () => {
  assertEquals(languageDescriptionForPath("assets/blob.cowboy-data"), null);
});
