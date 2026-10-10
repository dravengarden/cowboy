import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const html = await readFile(
  new URL("../index.html", import.meta.url), "utf8",
);
const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);
const themeSource = await readFile(
  new URL("./theme.ts", import.meta.url), "utf8",
);
const reviewSource = await readFile(
  new URL("./mobile/review/ReviewApp.tsx", import.meta.url), "utf8",
);
const repositorySource = await readFile(
  new URL("./mobile/review/ReviewRepository.tsx", import.meta.url), "utf8",
);
const fileTreeSource = await readFile(
  new URL("./mobile/review/ReviewFileTree.tsx", import.meta.url), "utf8",
);
const changesSource = await readFile(
  new URL("./mobile/review/ReviewChanges.tsx", import.meta.url), "utf8",
);
const exploreSource = await readFile(
  new URL("./explore/ExploreSurface.tsx", import.meta.url), "utf8",
);
const detentSheetSource = await readFile(
  new URL("../../components/app-shell/detent-sheet.tsx", import.meta.url), "utf8",
);
const connectionBannerSource = await readFile(
  new URL("../../components/app-shell/connection-banner.tsx", import.meta.url), "utf8",
);
const mobileNavigationSource = await readFile(
  new URL("../../components/app-shell/mobile-navigation.tsx", import.meta.url), "utf8",
);
const navShellSource = await readFile(
  new URL("../../components/app-shell/nav-shell.tsx", import.meta.url), "utf8",
);
const loginSource = await readFile(
  new URL("./auth/ProductLoginPage.tsx", import.meta.url), "utf8",
);
const deviceAuthSource = await readFile(
  new URL("./auth/DeviceAuthorizationPage.tsx", import.meta.url), "utf8",
);
const passkeyHtml = await readFile(
  new URL("../passkey.html", import.meta.url), "utf8",
);

test("wide standalone touch PWAs recover a missing iPad top inset", () => {
  assert(
    html.includes(
      "@media (display-mode: standalone) and (any-pointer: coarse) and (min-width: 700px) and (min-height: 700px)",
    ),
  );
  assert(
    html.includes(
      "--cowboy-system-top-clearance: max(env(safe-area-inset-top, 0px), 24px)",
    ),
  );
  assert(
    html.includes(
      "--cowboy-mobile-status-material-height: max(env(safe-area-inset-top, 0px), 24px)",
    ),
  );
  assert(
    html.includes(
      "html:has([data-detent-sheet='true']) [data-detent-sheet-chrome='true']",
    ),
  );
  assertEquals(
    html.includes(
      'apple-mobile-web-app-status-bar-style" content="black-translucent',
    ),
    false,
  );
});

test("only the iPad PWA floor paints a synthetic status strip", () => {
  // An iPhone PWA starts below its system-drawn status bar and reports a zero
  // inset. A floor there would add a blurred band under that bar.
  const floors = html.match(
    /--cowboy-mobile-status-material-height: max\([^;]*\);/g,
  ) ?? [];
  assertEquals(floors, [
    "--cowboy-mobile-status-material-height: max(env(safe-area-inset-top, 0px), 24px);",
  ]);
  assert(
    html.includes(
      "--cowboy-mobile-status-material-height: env(safe-area-inset-top, 0px);",
    ),
  );
  assertEquals(html.includes("cowboy-phone-standalone"), false);
  assertEquals(themeSource.includes("PhoneStandalone"), false);
  assertEquals(
    appSource.match(/var\(--cowboy-mobile-status-material-height\)/g)?.length,
    2,
  );
  assert(appSource.includes('pt: "var(--cowboy-system-top-clearance)"'));
  assert(reviewSource.includes('pt: "var(--cowboy-system-top-clearance)"'));
  assert(
    repositorySource.includes(
      "calc(var(--cowboy-system-top-clearance) + 14px)",
    ),
  );
  assert(
    fileTreeSource.includes("calc(var(--cowboy-system-top-clearance) + 18px)"),
  );
  assert(
    changesSource.includes("calc(var(--cowboy-system-top-clearance) + 18px)"),
  );
  assert(appSource.includes("frostedStatusChrome(t)"));
  assert(appSource.includes("data-mobile-status-strip-material={"));
});

test("transient Explore glass clears rather than overlaps iPad system chrome", () => {
  assert(
    exploreSource.includes(
      'top: "calc(var(--cowboy-system-top-clearance) + 8px)"',
    ),
  );
});

test("cover sheets and shared chrome consume the iPad top-clearance contract", () => {
  assert(
    detentSheetSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assertEquals(
    detentSheetSource.includes(
      'const SAFE_TOP = "env(safe-area-inset-top, 0px)"',
    ),
    false,
  );
  assert(
    connectionBannerSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assert(
    mobileNavigationSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assertEquals(
    navShellSource.includes('return "env(safe-area-inset-top, 0px)"'),
    false,
  );
  assert(
    navShellSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assert(
    loginSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assert(
    deviceAuthSource.includes(
      "var(--cowboy-system-top-clearance, env(safe-area-inset-top, 0px))",
    ),
  );
  assert(
    passkeyHtml.includes(
      "--cowboy-system-top-clearance: max(env(safe-area-inset-top, 0px), 24px)",
    ),
  );
});

test("the mobile Draft page clears the iPad status bar", () => {
  assert(
    appSource.includes(
      'data-mobile-drawer-surface={mobile ? "true" : undefined}\n                        sx={{ flex: 1, minHeight: 0, minWidth: 0, position: "relative", pointerEvents: "auto", pt: mobile && navbarAtBottom ? "var(--cowboy-system-top-clearance)" : 0 }}>',
    ),
  );
});
