import { assertEquals } from "jsr:@std/assert";
import {
  adminPasswordAcceptable,
  assessAdminPassword,
  looksLikePasswordManagerSecret,
} from "./passwordStrength.ts";

Deno.test("123123 is immediately too weak", () => {
  const assessed = assessAdminPassword("123123", "draven");
  assertEquals(assessed.level, "weak");
  assertEquals(assessed.label, "Too weak");
  assertEquals(assessed.acceptable, false);
  assertEquals(assessed.checks.length, false);
  assertEquals(assessed.checks.lower, false);
  assertEquals(assessed.checks.upper, false);
  assertEquals(assessed.checks.digit, true);
});

Deno.test("Chrome and Apple generated secrets are accepted", () => {
  assertEquals(looksLikePasswordManagerSecret("xidneh-bintun-zygfew"), true);
  assertEquals(adminPasswordAcceptable("xidneh-bintun-zygfew", "draven"), true);
  assertEquals(adminPasswordAcceptable("kL9mNp2qRs4tUv7", "draven"), true);
  assertEquals(adminPasswordAcceptable("Wq3p-Lm8n-Ks2xY", "draven"), true);
  assertEquals(assessAdminPassword("xidneh-bintun-zygfew", "draven").level, "strong");
  assertEquals(assessAdminPassword("kL9mNp2qRs4tUv7", "draven").level, "strong");
  assertEquals(assessAdminPassword("Wq3p-Lm8n-Ks2xY", "draven").level, "strong");
});
