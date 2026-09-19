import { assert, assertEquals } from "jsr:@std/assert";

const html = await Deno.readTextFile(
  new URL("../index.html", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const themeSource = await Deno.readTextFile(
  new URL("./theme.ts", import.meta.url),
);
const reviewSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewApp.tsx", import.meta.url),
);
const repositorySource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewRepository.tsx", import.meta.url),
);
const fileTreeSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewFileTree.tsx", import.meta.url),
);
const changesSource = await Deno.readTextFile(
  new URL("./mobile/review/ReviewChanges.tsx", import.meta.url),
);
const exploreSource = await Deno.readTextFile(
  new URL("./explore/ExploreSurface.tsx", import.meta.url),
);
const detentSheetSource = await Deno.readTextFile(
  new URL("../../components/app-shell/detent-sheet.tsx", import.meta.url),
);
const connectionBannerSource = await Deno.readTextFile(
  new URL("../../components/app-shell/connection-banner.tsx", import.meta.url),
);
const mobileNavigationSource = await Deno.readTextFile(
  new URL("../../components/app-shell/mobile-navigation.tsx", import.meta.url),
);
const navShellSource = await Deno.readTextFile(
  new URL("../../components/app-shell/nav-shell.tsx", import.meta.url),
);
const loginSource = await Deno.readTextFile(
  new URL("./auth/ProductLoginPage.tsx", import.meta.url),
);
const deviceAuthSource = await Deno.readTextFile(
  new URL("./auth/DeviceAuthorizationPage.tsx", import.meta.url),
);
const passkeyHtml = await Deno.readTextFile(
  new URL("../passkey.html", import.meta.url),
);

Deno.test("wide standalone touch PWAs recover a missing iPad top inset", () => {
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

Deno.test("only the iPad PWA floor paints a synthetic status strip", () => {
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

Deno.test("transient Explore glass clears rather than overlaps iPad system chrome", () => {
  assert(
    exploreSource.includes(
      'top: "calc(var(--cowboy-system-top-clearance) + 8px)"',
    ),
  );
});

Deno.test("cover sheets and shared chrome consume the iPad top-clearance contract", () => {
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
