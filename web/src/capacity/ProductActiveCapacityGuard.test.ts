import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const root = new URL("../", import.meta.url);

test("active capacity stays compact, pushed, and responsive across shells", async () => {
  const guard = await readFile(
    new URL("capacity/ProductActiveCapacityGuard.tsx", root), "utf8",
  );
  const gate = await readFile(new URL("auth/ProductAuthGate.tsx", root), "utf8");
  const topbar = await readFile(
    new URL("desktop/DesktopTopBarControls.tsx", root), "utf8",
  );

  assert(guard.includes("state.activeCapacity"));
  assertEquals(guard.includes("setInterval"), false);
  assert(guard.includes('capacity.status === "active"'));
  assert(guard.includes("This view stays read-only while it waits fairly"));
  assert(guard.includes("Close a duplicate tab or window"));
  assert(guard.includes("ProductSessionCapacityPanel"));
  assert(guard.includes("env(safe-area-inset-top"));
  assert(gate.includes("<ProductActiveCapacityGuard />"));
  assert(topbar.includes("setProductCapacityAlertHost"));
  assert(topbar.includes("data-product-capacity-alert-host"));
  assert(topbar.includes("data-desktop-topbar-action='capacity'"));
});
