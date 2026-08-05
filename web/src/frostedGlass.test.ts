import { assertEquals } from "jsr:@std/assert";
import { FROSTED_PILL_DROP_SHADOW_GEOMETRY } from "./floatingOverlayPolicy";
import { mobileTranscriptActivitySurfaceGap } from "./mobileComposerPrimitives";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);
const geometrySource = await Deno.readTextFile(
  new URL("./floatingComposerGeometry.ts", import.meta.url),
);
const turnStatusSource = await Deno.readTextFile(
  new URL("./TurnStatusOverlay.tsx", import.meta.url),
);
const permissionSource = await Deno.readTextFile(
  new URL("./PermissionOverlay.tsx", import.meta.url),
);

Deno.test("floating composer stack has one border-box geometry owner", () => {
  assertEquals(
    transcriptSource.includes('pb: bottomInset ?? "12px"'),
    true,
  );
  assertEquals(
    [transcriptSource, appSource, turnStatusSource, permissionSource].some(
      (source) => source.includes("--awaiting-h"),
    ),
    false,
  );
  assertEquals(
    geometrySource.includes('observer.observe(element, { box: "border-box" })'),
    true,
  );
  assertEquals(geometrySource.includes('"--floating-stack-h"'), true);
  assertEquals(geometrySource.includes('"--transcript-bottom-inset"'), true);
  assertEquals(
    appSource.includes('bottomInset="var(--transcript-bottom-inset, 0px)"'),
    true,
  );
  assertEquals(turnStatusSource.includes('position: "relative"'), true);
  assertEquals(permissionSource.includes('position: "relative"'), true);
});

Deno.test("frosted pill elevation stays close to the floating surface", () => {
  assertEquals(FROSTED_PILL_DROP_SHADOW_GEOMETRY, "0 5px 16px -10px");
});

Deno.test("focused mobile composer owns a real frosted material", () => {
  assertEquals(
    composerSource.includes('backdropFilter: "blur(24px) saturate(140%)"'),
    true,
  );
  assertEquals(
    composerSource.includes('WebkitBackdropFilter: "blur(24px) saturate(140%)"'),
    true,
  );
});

Deno.test("live Mobile thought surfaces breathe above the composer boundary", () => {
  assertEquals(mobileTranscriptActivitySurfaceGap, 6);
  assertEquals(
    transcriptSource.includes(
      "pb: touch && streaming\n          ? `${mobileTranscriptActivitySurfaceGap}px`",
    ),
    true,
  );
  assertEquals(transcriptSource.includes("touch={!desktop}"), true);
});
