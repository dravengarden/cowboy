import { assert, assertEquals } from "jsr:@std/assert";

Deno.test("setup page never asks for a machine id", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assertEquals(source.includes("Machine id"), false);
  assertEquals(source.includes("machine_id: id"), false);
  assertEquals(source.includes("slugMachineId"), false);
  assert(source.includes("cowboy register ${issued.origin}"));
  assert(source.includes("`${command} --background`"));
  assert(source.includes("Cowboy assigns the machine id"));
  assert(source.includes('JSON.stringify(name ? { display_name: name } : {})'));
});

Deno.test("setup page explains foreground and background registration", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assert(source.includes("Runs in this terminal. Keep it open"));
  assert(source.includes("per-user background service"));
  assert(source.includes("MACHINE_SETUP_DOCS_URL"));
  assert(source.includes("MACHINE_SETUP_SKILL_URL"));
  assert(source.includes('label="Setup guide"'));
  assert(source.includes('label="Setup skill"'));
  assert(source.includes("{label} · Coming soon"));
});

Deno.test("setup page centers against the dynamic viewport", async () => {
  const page = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  const gate = await Deno.readTextFile(new URL("MachineSetupGate.tsx", import.meta.url));
  assert(page.includes('minHeight: "100dvh"'));
  assert(page.includes('justifyContent: "center"'));
  assert(gate.includes('minHeight: "100dvh"'));
});

Deno.test("setup settings include theme, typeface, size, passkeys, and sign out", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assert(source.includes("FONT_PRESETS"));
  assert(source.includes("FONT_SCALE_PRESETS"));
  assert(source.includes("setFontVariant"));
  assert(source.includes("setFontScale"));
  assert(source.includes("ProductPasskeysPanel"));
  assert(source.includes("ProductAccountMenu"));
  assert(source.includes("Typeface"));
  assertEquals(source.includes("<Drawer"), false);
});

Deno.test("product sign out keeps destructive action styling", async () => {
  const source = await Deno.readTextFile(
    new URL("../auth/ProductAccountMenu.tsx", import.meta.url),
  );
  assert(source.includes('color="error"'));
  assertEquals(source.includes('color="inherit"'), false);
});

Deno.test("enrollment expiry is live and offers a fresh code", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assert(source.includes("expires_at_ms"));
  assert(source.includes("remainingSeconds"));
  assert(source.includes("This one-time token has expired."));
  assert(source.includes("Create a new code"));
  assert(source.includes("Date.now() < issued.expires_at_ms"));
});

Deno.test("issued enrollment can be discarded before returning to details", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assert(source.includes('method: "DELETE"'));
  assert(source.includes("body: JSON.stringify({ token })"));
  assert(source.includes("setIssued(null)"));
  assert(source.includes("Discard this code and edit the computer name."));
});

Deno.test("enrollment token is masked by default with an explicit reveal control", async () => {
  const source = await Deno.readTextFile(new URL("MachineSetupPage.tsx", import.meta.url));
  assert(source.includes("maskSecret(value)"));
  assert(source.includes('"*".repeat(value.length - visible)'));
  assert(source.includes('aria-label={revealed ? "Hide enrollment token" : "Show enrollment token"}'));
  assert(source.includes("<VisibilityOutlined"));
  assert(source.includes("<VisibilityOffOutlined"));
  assert(source.includes("navigator.clipboard.writeText(value)"));
  assert(source.includes("secret\n              />"));
});
