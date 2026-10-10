import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { SHELL_COMMENT_PATTERN } from "./shellLanguage.ts";

function comment(source: string): string | undefined {
  return SHELL_COMMENT_PATTERN.exec(source)?.[0];
}

test("shell comment boundary preserves Nix flake fragments", () => {
  assertEquals(comment("nix build '.#nixosConfigurations.hawk.pkgs.cowboy-web'"), undefined);
  assertEquals(comment("curl https://host/path#fragment"), undefined);
});

test("shell comment boundary retains real comments", () => {
  assert(comment("cmd && # explain")?.includes("# explain"));
  assertEquals(comment("# heading"), "# heading");
});
