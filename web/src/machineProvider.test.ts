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

Deno.test("maps the Codex DeepSeek runtime to the managed Codex slots", () => {
  assertEquals(
    machineProviderAvailable("codex-deepseek", [{
      id: { kind: "provider_cli", slot: "codex" },
      state: "active",
      auth: "signed_in",
    }]),
    false,
  );
  assertEquals(
    machineProviderAvailable("codex-deepseek", [{
      id: { kind: "provider_cli", slot: "codex" },
      state: "active",
      auth: "signed_out",
    }, {
      id: { kind: "provider_adapter", slot: "codex" },
      state: "active",
    }, {
      id: { kind: "provider_adapter", slot: "codex-deepseek" },
      state: "active",
    }]),
    true,
  );
});

Deno.test("maps Claude DeepSeek to isolated gateway and shared Claude adapter", () => {
  assertEquals(
    machineProviderAvailable("claude-deepseek", [{
      id: { kind: "provider_cli", slot: "claude" },
      state: "active",
      auth: "signed_in",
    }, {
      id: { kind: "provider_adapter", slot: "claude" },
      state: "active",
    }]),
    false,
  );
  assertEquals(
    machineProviderAvailable("claude-deepseek", [{
      id: { kind: "provider_cli", slot: "claude" },
      state: "active",
      auth: "signed_out",
    }, {
      id: { kind: "provider_adapter", slot: "claude" },
      state: "active",
    }, {
      id: { kind: "provider_adapter", slot: "claude-deepseek" },
      state: "active",
    }]),
    true,
  );
});

Deno.test("maps Reasonix to its native CLI and isolated gateway", () => {
  const cli = {
    id: { kind: "provider_cli", slot: "reasonix" },
    state: "active",
    auth: "unsupported",
  };
  const gateway = {
    id: { kind: "provider_adapter", slot: "reasonix-deepseek" },
    state: "active",
  };
  assertEquals(machineProviderAvailable("reasonix-deepseek", [cli]), false);
  assertEquals(machineProviderAvailable("reasonix-deepseek", [gateway]), false);
  assertEquals(machineProviderAvailable("reasonix-deepseek", [cli, gateway]), true);
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
