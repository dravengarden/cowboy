import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const storeSource = await readFile(
  new URL("./store.ts", import.meta.url), "utf8",
);

test("Machines can be deleted only through the current Service API", () => {
  assert(appSource.includes("function MachinesContent"));
  assert(appSource.includes("/revoke`"));
  assert(appSource.includes("Delete from this Cowboy Service"));
  assert(appSource.includes("Other Cowboy Services"));
  assert(appSource.includes("snapshot.machines"));
  assert(!/setMachines\(\(current\) =>\s*current\.filter/u.test(appSource));
});

test("deleting the last Machine waits for the authoritative pushed snapshot", async () => {
  const gateSource = await readFile(
    new URL("./setup/MachineSetupGate.tsx", import.meta.url), "utf8",
  );
  assert(appSource.includes("deletingLastMachine"));
  assert(appSource.includes("This is the last registered computer."));
  assert(!appSource.includes("MACHINE_SETUP_REFRESH_EVENT"));
  assert(gateSource.includes("snapshot.machinesLoaded"));
  assert(!gateSource.includes("setInterval"));
  assert(storeSource.includes('case "machines"'));
  assert(gateSource.includes("startViewTransition"));
  assert(gateSource.includes("machine-setup-enter"));
});
