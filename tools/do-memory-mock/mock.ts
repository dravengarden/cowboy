/**
 * Offline stand-in for `wrangler dev` when the Cloudflare toolchain is not
 * installed. Same fixture as the Durable Object worker.
 */

const SESSION_COUNT = 17;
const TERMINAL_COUNT = 3;
const RAW_OUTPUT_BYTES = 2_580_000;
const ISOLATE_BUDGET = 128 * 1024 * 1024;
const TARGET_BUDGET = 100 * 1024 * 1024;

const liveFrame = JSON.stringify({
  sessionId: "session-0",
  seq: 0,
  kind: "tool_call",
  content: [{ type: "raw_output", text: "ok" }],
});

const hotLogBytes = SESSION_COUNT * liveFrame.length;
const sessionMetaBytes = JSON.stringify(
  Array.from({ length: SESSION_COUNT }, (_, index) => ({
    id: `session-${index}`,
    status: index === 0 ? "busy" : "running",
  })),
).length;
const workingSetBytes = sessionMetaBytes + hotLogBytes + liveFrame.length;
const report = {
  sessions: SESSION_COUNT,
  terminals: TERMINAL_COUNT,
  compactLiveBytes: liveFrame.length,
  sharedFrame: true,
  hotLogBytes,
  sessionMetaBytes,
  rawFanoutWouldHaveBeen: RAW_OUTPUT_BYTES * TERMINAL_COUNT,
  estimatedWorkingSetBytes: workingSetBytes,
  isolateBudgetBytes: ISOLATE_BUDGET,
  targetBudgetBytes: TARGET_BUDGET,
  fitsTarget: workingSetBytes < TARGET_BUDGET,
  fitsIsolate: workingSetBytes < ISOLATE_BUDGET,
  droppedRawOutput: !liveFrame.includes("rawOutput"),
};

console.log(JSON.stringify(report, null, 2));

if (!report.fitsTarget || !report.droppedRawOutput || !report.sharedFrame) {
  Deno.exit(1);
}
