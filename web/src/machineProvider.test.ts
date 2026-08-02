import { assertEquals } from "jsr:@std/assert";
import { machineProviderAvailable } from "./machineProvider.ts";

Deno.test("maps the claude-code session provider to the claude CLI slot", () => {
  assertEquals(
    machineProviderAvailable("claude-code", [{
      id: { kind: "provider_cli", slot: "claude" },
      state: "active",
      auth: "signed_in",
    }]),
    true,
  );
});

Deno.test("requires an active and confirmed signed-in provider", () => {
  assertEquals(
    machineProviderAvailable("gemini", [{
      id: { kind: "provider_cli", slot: "gemini" },
      state: "active",
      auth: "signed_out",
    }]),
    false,
  );
  assertEquals(
    machineProviderAvailable("gemini", [{
      id: { kind: "provider_cli", slot: "gemini" },
      state: "active",
      auth: "signed_in",
    }]),
    false,
  );
  assertEquals(
    machineProviderAvailable("gemini", [{
      id: { kind: "provider_cli", slot: "gemini" },
      state: "active",
      auth: "signed_in",
      detail: "Gemini API key",
    }]),
    true,
  );
});
