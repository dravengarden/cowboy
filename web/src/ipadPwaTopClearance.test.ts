import { assert, assertEquals } from "jsr:@std/assert";
import {
  PHONE_STANDALONE_ROOT_CLASS,
  prefersPhoneStandaloneStatusShelf,
  syncPhoneStandaloneRootClass,
} from "./platform.ts";

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
      "@media (display-mode: standalone) and (any-pointer: coarse) and (min-width: 700px)",
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

Deno.test("phone standalone status material stays separate from iPad clearance", () => {
  assert(
    html.includes(`:root.${PHONE_STANDALONE_ROOT_CLASS}`),
  );
  assert(
    html.includes(
      "--cowboy-mobile-status-material-height: max(env(safe-area-inset-top, 0px), 32px)",
    ),
  );
  assertEquals(
    appSource.match(/var\(--cowboy-mobile-status-material-height\)/g)?.length,
    2,
  );
  assert(themeSource.includes("syncPhoneStandaloneRootClass();"));
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

Deno.test("only a coarse iPhone standalone PWA gets the status shelf", () => {
  const iPhone = {
    appleStandalone: true,
    coarsePointer: true,
    screenWidth: 393,
    screenHeight: 852,
  };
  assertEquals(prefersPhoneStandaloneStatusShelf(iPhone), true);
  assertEquals(
    prefersPhoneStandaloneStatusShelf({
      ...iPhone,
      screenWidth: 852,
      screenHeight: 393,
    }),
    true,
  );
  assertEquals(
    prefersPhoneStandaloneStatusShelf({ ...iPhone, appleStandalone: false }),
    false,
  );
  assertEquals(
    prefersPhoneStandaloneStatusShelf({
      ...iPhone,
      screenWidth: 744,
      screenHeight: 1133,
    }),
    false,
  );

  const classes = new Set<string>();
  assertEquals(
    syncPhoneStandaloneRootClass({
      documentElement: {
        classList: {
          toggle(name, force): boolean {
            if (force) classes.add(name);
            else classes.delete(name);
            return force;
          },
        },
      },
    }, iPhone),
    true,
  );
  assertEquals(classes.has(PHONE_STANDALONE_ROOT_CLASS), true);
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
