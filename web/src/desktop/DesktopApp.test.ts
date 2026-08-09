import { assertEquals } from "jsr:@std/assert";

const desktopAppSource = await Deno.readTextFile(
  new URL("./DesktopApp.tsx", import.meta.url),
);
const workspaceControllerSource = await Deno.readTextFile(
  new URL("./DesktopWorkspaceController.tsx", import.meta.url),
);

Deno.test("desktop never mounts a page-wide keyboard target overlay", () => {
  assertEquals(desktopAppSource.includes("DesktopHintOverlay"), false);
  assertEquals(workspaceControllerSource.includes('"hint"'), false);
});
