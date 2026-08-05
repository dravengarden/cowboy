import { assertEquals } from "jsr:@std/assert";
import {
  FROSTED_PILL_DROP_SHADOW_GEOMETRY,
  TURN_STATUS_PILL_MIN_HEIGHT,
} from "./floatingOverlayPolicy";
import { mobileTranscriptTailGap } from "./mobileComposerPrimitives";

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
const transcriptActivitySource = await Deno.readTextFile(
  new URL("./TranscriptTurnActivity.tsx", import.meta.url),
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

Deno.test("a context reset becomes a full conversation start until new content", () => {
  assertEquals(
    transcriptSource.includes("data-transcript-context-boundary"),
    true,
  );
  assertEquals(
    transcriptSource.includes('data-conversation-empty-state={kind}'),
    true,
  );
  assertEquals(
    transcriptSource.includes(
      '<ConversationEmptyState kind="cleared" context={conversationContext} />',
    ),
    true,
  );
  assertEquals(transcriptSource.includes("New conversation"), true);
  assertEquals(transcriptSource.includes("Conversation cleared\n"), false);
});

Deno.test("judge progress is transcript-owned with a zero-jump pill handoff", () => {
  assertEquals(TURN_STATUS_PILL_MIN_HEIGHT, 36);
  assertEquals(
    transcriptActivitySource.includes(
      'data-transcript-turn-activity="judging"',
    ),
    true,
  );
  assertEquals(
    transcriptActivitySource.includes("TURN_STATUS_PILL_MIN_HEIGHT"),
    true,
  );
  assertEquals(
    turnStatusSource.includes("TURN_STATUS_PILL_MIN_HEIGHT"),
    true,
  );
  assertEquals(
    transcriptActivitySource.includes("data-composer-stack-slot"),
    false,
  );
  assertEquals(transcriptSource.includes('data-transcript-tail-row="judging"'), true);
  assertEquals(transcriptSource.includes("showJudging && ("), true);
  assertEquals(transcriptSource.includes("<TranscriptJudgingActivity />"), true);
  assertEquals(transcriptSource.includes("data-transcript-tail-clearance"), true);
  assertEquals(appSource.includes("judging={judging}"), true);
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

Deno.test("every Mobile transcript tail shares one external boundary gap", () => {
  assertEquals(mobileTranscriptTailGap, 6);
  assertEquals(
    transcriptSource.includes(
      "data-transcript-tail-clearance",
    ),
    true,
  );
  assertEquals(
    transcriptSource.includes("height: `${mobileTranscriptTailGap}px`"),
    true,
  );
  assertEquals(
    transcriptSource.includes("mobileTranscriptActivitySurfaceGap"),
    false,
  );
});

Deno.test("live thought shimmer crosses each glyph run once per cycle", () => {
  assertEquals(
    transcriptSource.includes("from { background-position: 100% 0; }"),
    true,
  );
  assertEquals(
    transcriptSource.includes("to   { background-position: 0% 0; }"),
    true,
  );
  assertEquals(
    transcriptSource.match(/backgroundRepeat: "no-repeat"/g)?.length,
    2,
  );
  assertEquals(transcriptSource.includes("background-position: -110% 0"), false);
});

Deno.test("thought indicators align to the inherited first-line box", () => {
  assertEquals(
    transcriptSource.includes("data-thought-step-indicator-lane"),
    true,
  );
  assertEquals(
    transcriptSource.includes('minHeight: "1lh"'),
    true,
  );
  assertEquals(
    transcriptSource.includes(
      "top: `calc(0.5lh - ${indicatorSize / 2}px)`,",
    ),
    true,
  );
  assertEquals(
    transcriptSource.includes(
      "gridTemplateColumns: `${indicatorSize}px minmax(0, 1fr)`,",
    ),
    true,
  );
  assertEquals(transcriptSource.includes('top: current ? "0.43em"'), false);
  assertEquals(transcriptSource.includes('top: "0.62em"'), false);
});
