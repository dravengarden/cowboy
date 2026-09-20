import { assertEquals } from "jsr:@std/assert";
import {
  machineConvergencePresentation,
  machineSupersessionPresentation,
  machineVersionPresentation,
} from "./machineVersions.ts";

Deno.test("Machine version rows distinguish health from release freshness", () => {
  assertEquals(
    machineVersionPresentation("0.145.0", "active", {
      latest_version: "0.146.0",
      available: true,
      source: "npm registry",
      checked_at_ms: 1,
      installable: false,
    }),
    {
      version: "Installed 0.145.0 · Latest 0.146.0",
      status: "Update available",
      tone: "warning",
    },
  );
  assertEquals(
    machineVersionPresentation("0.146.0", "active", {
      latest_version: "0.146.0",
      available: false,
      source: "npm registry",
      checked_at_ms: 1,
      installable: false,
    }),
    {
      version: "Installed 0.146.0 · Up to date",
      status: "Up to date",
      tone: "success",
    },
  );
});

Deno.test("unknown release state never claims a component is current", () => {
  assertEquals(machineVersionPresentation("1.13.0", "active"), {
    version: "Installed 1.13.0",
    status: "active",
    tone: "success",
  });
});

Deno.test("automatic convergence says what the Controller is doing", () => {
  const id = { kind: "zed_server", slot: "zed" };
  assertEquals(
    machineConvergencePresentation({ id, state: "pending" }, 0).status,
    "Updating automatically",
  );
  assertEquals(
    machineConvergencePresentation({ id, state: "verifying" }, 0).status,
    "Confirming the update",
  );
  assertEquals(
    machineConvergencePresentation({ id, state: "draining" }, 0),
    {
      status: "Updates when sessions finish",
      tone: "default",
      detail: "A running session still uses the installed generation",
    },
  );
  assertEquals(
    machineConvergencePresentation(
      { id, state: "retrying", next_attempt_at_ms: 4 * 60_000, detail: "probe failed" },
      60_000,
    ),
    { status: "Retrying in 3 min", tone: "warning", detail: "probe failed" },
  );
  // A retry that is already due reads as a plain retry, never "in 0 min".
  assertEquals(
    machineConvergencePresentation({ id, state: "retrying", next_attempt_at_ms: 10 }, 60_000).status,
    "Retrying",
  );
  assertEquals(
    machineConvergencePresentation({ id, state: "blocked", attempts: 4 }, 0),
    {
      status: "Update blocked",
      tone: "error",
      detail: "Stopped after 4 attempts against this exact release",
    },
  );
});

Deno.test("a converging component offers no action that would race the Controller", async () => {
  const app = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
  // Draining and blocked keep the per-component action: both are exactly where
  // a person still decides.
  const guard = app.indexOf("const converging = convergence !== undefined &&");
  const drainingAndBlocked = app.slice(guard, guard + 200);
  assertEquals(drainingAndBlocked.includes('convergence.state !== "draining"'), true);
  assertEquals(drainingAndBlocked.includes('convergence.state !== "blocked"'), true);
  assertEquals(app.includes("{componentPending && !converging && ("), true);
});

Deno.test("a Plugin-served slot offers no legacy update to press", async () => {
  const presentation = machineSupersessionPresentation("claude-code");
  assertEquals(presentation.status, "Served by claude-code");
  assertEquals(presentation.tone, "default");

  const app = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
  // The npm action is the unpinned `@latest` path; it must not be offered for a
  // slot an installed Plugin already serves from its pinned generation.
  assertEquals(
    app.includes("update.installable && supersession === undefined"),
    true,
  );
});

Deno.test("a Service-managed Machine offers no Plugin lifecycle action", async () => {
  const management = await Deno.readTextFile(
    new URL("./ProviderManagement.tsx", import.meta.url),
  );
  // Blocking the capability removes the button entirely (ProviderSurface
  // renders nothing for a blocked action), so a managed Machine cannot be
  // installed to, upgraded or uninstalled from a client.
  const blocked = management.slice(
    management.indexOf("const SERVICE_MANAGED_EFFECTS"),
    management.indexOf("type ProviderManagementProps"),
  );
  for (const capability of ["install_on_machine", "upgrade_on_machine", "request_uninstall_plan"]) {
    assertEquals(blocked.includes(`"${capability}"`), true, capability);
  }
  assertEquals(
    management.includes(`machine?.plugin_lifecycle === "managed"`),
    true,
  );
});
