import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const desktopAppSource = await readFile(
  new URL("./DesktopApp.tsx", import.meta.url), "utf8",
);
const workspaceControllerSource = await readFile(
  new URL("./DesktopWorkspaceController.tsx", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("../App.tsx", import.meta.url), "utf8",
);

test("desktop never mounts a page-wide keyboard target overlay", () => {
  assertEquals(desktopAppSource.includes("DesktopHintOverlay"), false);
  assertEquals(workspaceControllerSource.includes('"hint"'), false);
});

test("the open session owns selected material in rail and collapsed layouts", () => {
  assertEquals(
    appSource.includes(
      `...(desktop && s.id === activeId && {`,
    ),
    true,
  );
  assertEquals(
    appSource.includes(
      `"&&.Mui-selected, &&[data-desktop-current='true']": {`,
    ),
    true,
  );
  assertEquals(
    appSource.includes(
      `"&&.Mui-selected:hover, &&[data-desktop-current='true']:hover": {`,
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
