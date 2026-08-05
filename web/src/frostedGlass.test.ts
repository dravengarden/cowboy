import { assertEquals } from "jsr:@std/assert";
import {
  FROSTED_PILL_DROP_SHADOW_GEOMETRY,
  FLOATING_OVERLAY_BOUNDARY_GAP_PX,
} from "./floatingOverlayPolicy";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const appSource = await Deno.readTextFile(
  new URL("./App.tsx", import.meta.url),
);

Deno.test("floating composer overlays keep a narrow boundary seam", () => {
  assertEquals(FLOATING_OVERLAY_BOUNDARY_GAP_PX, 4);
  assertEquals(
    transcriptSource.includes(
      "`calc(${bottomInset} + var(--awaiting-h, 0px))`",
    ),
    true,
  );
  assertEquals(
    transcriptSource.includes(
      "`calc(${bottomInset} + var(--awaiting-h, 0px) + 8px)`",
    ),
    false,
  );
  assertEquals(
    appSource.includes(
      '"&:not(:has([data-turn-status-overlay], [data-permission-overlay]))"',
    ),
    true,
  );
  assertEquals(appSource.includes('"--awaiting-h": "0px"'), true);
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
