import { assert, assertEquals } from "jsr:@std/assert";

const html = await Deno.readTextFile(
  new URL("../index.html", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
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
  assertEquals(
    html.includes(
      'apple-mobile-web-app-status-bar-style" content="black-translucent',
    ),
    false,
  );
});

Deno.test("Transcript keeps the real inset while other surfaces use iPad clearance", () => {
  assert(
    appSource.includes(
      'height: navbarAtBottom ? "env(safe-area-inset-top, 0px)"',
    ),
  );
  assert(
    appSource.includes(
      'topInset={navbarAtBottom ? "env(safe-area-inset-top, 0px)"',
    ),
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
});

Deno.test("transient Explore glass clears rather than overlaps iPad system chrome", () => {
  assert(
    exploreSource.includes(
      'top: "calc(var(--cowboy-system-top-clearance) + 8px)"',
    ),
  );
});
