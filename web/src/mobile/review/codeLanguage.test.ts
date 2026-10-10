import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { languageDescriptionForPath } from "./codeLanguage.ts";

test("Code language routing recognizes common repository files", () => {
  assertEquals(languageDescriptionForPath("src/main.rs")?.name, "Rust");
  assertEquals(languageDescriptionForPath("web/src/main.tsx")?.name, "TSX");
  assertEquals(languageDescriptionForPath("Cargo.lock")?.name, "TOML");
  assertEquals(languageDescriptionForPath("docs/guide.md")?.name, "Markdown");
  assertEquals(
    languageDescriptionForPath("systemd/cowboy.service")?.name,
    "Properties files",
  );
});

test("Code language routing uses Zed shebang detection for extensionless files", () => {
  assertEquals(
    languageDescriptionForPath(
      "scripts/shadow-fixture-smoke",
      "#!/usr/bin/env bash\nset -euo pipefail\n",
    )?.name,
    "Shell",
  );
});

test("Code language routing keeps path detection ahead of shebang detection", () => {
  assertEquals(
    languageDescriptionForPath("scripts/run.js", "#!/usr/bin/env bash\n")?.name,
    "JavaScript",
  );
});

test("Code language routing leaves unknown binary-shaped names plain", () => {
  assertEquals(
    languageDescriptionForPath("assets/blob.cowboy-data", "binary data\n"),
    null,
  );
});
