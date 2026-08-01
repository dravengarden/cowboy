import { assertEquals } from "jsr:@std/assert";
import { machineVersionPresentation } from "./machineVersions.ts";

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
