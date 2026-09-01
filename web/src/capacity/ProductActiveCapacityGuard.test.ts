import { assert, assertEquals } from "jsr:@std/assert";

const root = new URL("../", import.meta.url);

Deno.test("active capacity stays compact, pushed, and responsive across shells", async () => {
  const guard = await Deno.readTextFile(
    new URL("capacity/ProductActiveCapacityGuard.tsx", root),
  );
  const gate = await Deno.readTextFile(new URL("auth/ProductAuthGate.tsx", root));
  const topbar = await Deno.readTextFile(
    new URL("desktop/DesktopTopBarControls.tsx", root),
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
