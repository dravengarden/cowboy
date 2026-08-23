import { assertEquals } from "jsr:@std/assert";

const desktopAppSource = await Deno.readTextFile(
  new URL("./DesktopApp.tsx", import.meta.url),
);
const workspaceControllerSource = await Deno.readTextFile(
  new URL("./DesktopWorkspaceController.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("../App.tsx", import.meta.url),
);

Deno.test("desktop never mounts a page-wide keyboard target overlay", () => {
  assertEquals(desktopAppSource.includes("DesktopHintOverlay"), false);
  assertEquals(workspaceControllerSource.includes('"hint"'), false);
});

Deno.test("the open session owns selected material in rail and collapsed layouts", () => {
  assertEquals(
    appSource.includes(
      `...(desktop && s.id === activeId && {`,
    ),
    true,
  );
  assertEquals(
    appSource.includes(
      `"& [data-desktop-region='sessions.list'] [data-desktop-item][data-desktop-current='true']"`,
    ),
    false,
  );
  assertEquals(appSource.includes("data-desktop-active-session"), true);
  assertEquals(
    /surface === "desktop" && sessionsInDrawer &&\s*!drawerOpen &&/u.test(
      appSource,
    ),
    true,
  );
});
