import { assertEquals } from "jsr:@std/assert";
import {
  FROSTED_PILL_DROP_SHADOW_GEOMETRY,
  TURN_STATUS_PILL_MIN_HEIGHT,
} from "./floatingOverlayPolicy";
import { mobileTranscriptTailGap } from "./mobileComposerPrimitives";

const composerSource = await Deno.readTextFile(
  new URL("./Composer.tsx", import.meta.url),
);
const composerSurfaceSource = await Deno.readTextFile(
  new URL("./mobileComposerSurface.ts", import.meta.url),
);
const transcriptSource = await Deno.readTextFile(
  new URL("./Transcript.tsx", import.meta.url),
);
const providerTranscriptSource = await Deno.readTextFile(
  new URL("./ProviderTranscript.tsx", import.meta.url),
);
const exploreSurfaceSource = await Deno.readTextFile(
  new URL("./explore/ExploreSurface.tsx", import.meta.url),
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
    appSource.includes('"var(--transcript-bottom-inset, 0px)"'),
    true,
  );
  assertEquals(turnStatusSource.includes('position: "relative"'), true);
  assertEquals(permissionSource.includes('position: "relative"'), true);
});

Deno.test("frosted pill elevation stays close to the floating surface", () => {
  assertEquals(FROSTED_PILL_DROP_SHADOW_GEOMETRY, "0 5px 16px -10px");
});

Deno.test("permission actions follow the global reading font scale", () => {
  assertEquals(permissionSource.includes('fontSize: "1rem"'), true);
  assertEquals(permissionSource.includes('fontSize: "0.875rem"'), true);
  assertEquals(
    permissionSource.includes("fontSize: { xs: 15, sm: 14 }"),
    false,
  );
});

Deno.test("a context reset becomes a full conversation start until new content", () => {
  assertEquals(
    transcriptSource.includes("data-transcript-context-boundary"),
    true,
  );
  assertEquals(
    transcriptSource.includes("data-conversation-empty-state={kind}"),
    true,
  );
  assertEquals(
    /<ConversationEmptyState\s+kind="cleared"\s+context=\{conversationContext\}\s*\/>/u
      .test(transcriptSource),
    true,
  );
  assertEquals(transcriptSource.includes("New conversation"), true);
  assertEquals(transcriptSource.includes("Conversation cleared\n"), false);
});

Deno.test("the Explore page dock yields to the focused mobile composer", () => {
  assertEquals(
    exploreSurfaceSource.includes('data-mobile-page-dock="true"'),
    true,
  );
  assertEquals(appSource.includes("data-mobile-page-dock='true'"), true);
  assertEquals(
    appSource.includes(
      "[data-mobile-primary-composer='true'][data-mobile-keyboard-open='true'] [data-mobile-editor-area]:focus-within",
    ),
    true,
  );
  assertEquals(transcriptSource.includes("context.reasoning"), true);
  assertEquals(transcriptSource.includes("Reasoning ·"), true);
  assertEquals(
    transcriptSource.includes("data-conversation-empty-settings"),
    true,
  );
});

Deno.test("reconnect activity is transcript-owned and judge UI stays retired", () => {
  assertEquals(TURN_STATUS_PILL_MIN_HEIGHT, 36);
  assertEquals(
    transcriptActivitySource.includes('data-transcript-turn-activity="reconnecting"'),
    true,
  );
  assertEquals(
    transcriptActivitySource.includes("TranscriptReconnectingActivity"),
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
  assertEquals(
    transcriptSource.includes('data-transcript-tail-row="judging"'),
    false,
  );
  assertEquals(
    transcriptSource.includes('data-transcript-tail-row="reconnecting"'),
    true,
  );
  assertEquals(turnStatusSource.includes('label: "Reconnecting…"'), false);
  assertEquals(transcriptSource.includes("showJudging && ("), false);
  assertEquals(
    transcriptSource.includes("<TranscriptJudgingActivity />"),
    false,
  );
  assertEquals(
    transcriptSource.includes("data-transcript-tail-clearance"),
    true,
  );
  assertEquals(appSource.includes("judging={judging}"), false);
  assertEquals(transcriptActivitySource.includes("Judging…"), false);
});

Deno.test("mobile composer chrome restores resting frost without a swipe filter", () => {
  assertEquals(appSource.includes("frostedChrome"), true);
  assertEquals(appSource.includes("mobileFrostFollowRef"), true);
  assertEquals(
    appSource.includes("data-mobile-backdrop-chrome=\"true\""),
    true,
  );
});

Deno.test("focused mobile composer owns a real frosted material", () => {
  assertEquals(
    composerSurfaceSource.includes(
      "return theme.palette.background.paper;",
    ),
    true,
  );
  assertEquals(
    composerSource.includes(
      "\"&[data-mobile-keyboard-open='true'] [data-mobile-editor-area], &[data-mobile-keyboard-open='true'] [data-mobile-action-row]",
    ),
    true,
  );
  assertEquals(
    composerSurfaceSource.includes("backdropFilter"),
    false,
  );
  assertEquals(
    composerSurfaceSource.includes("WebkitBackdropFilter"),
    false,
  );
  assertEquals(
    composerSource.match(/mobileFocusedComposerSurfaceSx/g)?.length,
    4,
  );
  assertEquals(composerSource.includes("mobileFocusedComposerFill"), true);
});

Deno.test("every Mobile transcript tail shares one external boundary gap", () => {
  assertEquals(mobileTranscriptTailGap, 12);
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

Deno.test("Provider thought shimmer crosses each glyph run once per cycle", () => {
  assertEquals(
    providerTranscriptSource.includes(
      "from { background-position: 100% 0; }",
    ),
    true,
  );
  assertEquals(
    providerTranscriptSource.includes("to { background-position: 0% 0; }"),
    true,
  );
  assertEquals(
    providerTranscriptSource.match(/backgroundRepeat: "no-repeat"/g)?.length,
    2,
  );
  assertEquals(
    providerTranscriptSource.includes("background-position: -110% 0"),
    false,
  );
});

Deno.test("thought indicators align to the inherited first-line box", () => {
  assertEquals(
    providerTranscriptSource.includes("data-thought-step-indicator-lane"),
    true,
  );
  assertEquals(
    providerTranscriptSource.includes('minHeight: "1lh"'),
    true,
  );
  assertEquals(
    providerTranscriptSource.includes(
      "top: `calc(0.5lh - ${geometry.size / 2}px)`,",
    ),
    true,
  );
  assertEquals(
    providerTranscriptSource.includes(
      "`${geometry.size}px minmax(0, 1fr)`,",
    ),
    true,
  );
  assertEquals(
    providerTranscriptSource.includes('top: current ? "0.43em"'),
    false,
  );
  assertEquals(providerTranscriptSource.includes('top: "0.62em"'), false);
});
