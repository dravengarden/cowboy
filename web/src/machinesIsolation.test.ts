import { assert } from "jsr:@std/assert";

const appSource = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));

Deno.test("Machines can be deleted only through the current Service API", () => {
  assert(appSource.includes("function MachinesContent"));
  assert(appSource.includes("/revoke`"));
  assert(appSource.includes("Delete from this Cowboy Service"));
  assert(appSource.includes("Other Cowboy Services"));
  assert(appSource.includes("setMachines((current) => current.filter"));
});

Deno.test("deleting the last Machine warns and refreshes the setup gate immediately", async () => {
  const gateSource = await Deno.readTextFile(
    new URL("./setup/MachineSetupGate.tsx", import.meta.url),
  );
  assert(appSource.includes("deletingLastMachine"));
  assert(appSource.includes("This is the last registered computer."));
  assert(appSource.includes("MACHINE_SETUP_REFRESH_EVENT"));
  assert(gateSource.includes("MACHINE_SETUP_REFRESH_EVENT"));
  assert(gateSource.includes("startViewTransition"));
  assert(gateSource.includes("machine-setup-enter"));
});
