/** Profile the Desktop App under a production-sized workload in an already
 * running Chrome (disposable context, fictitious origin, no credentials; see
 * cdp-fixture.ts). Prints per-scenario frame/long-task measurements and the
 * hottest functions by self time, and writes `.cpuprofile` files to OUT.
 *
 * Usage: bun tools/desktop-perf-profile.ts <endpoint> <out-dir> [perf-data.json]
 */
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { openFixturePage } from "./cdp-fixture.ts";

const [endpoint = "", out = "/tmp/cowboy-desktop-perf", data] = process.argv
  .slice(2);
const only = process.env.PERF_SCENARIOS?.split(",");
await mkdir(out, { recursive: true });
const files: Record<string, { type: string; body: Uint8Array }> = {};
if (data) {
  files["/perf-data.json"] = {
    type: "application/json",
    body: await readFile(data),
  };
}

const page = await openFixturePage(
  endpoint,
  "desktop-perf",
  (token) =>
    `<!doctype html><script>
localStorage.setItem("cowboy:theme-system-default-v1", "1");
localStorage.setItem("cowboy-theme-mode", ${
      JSON.stringify(process.env.PERF_THEME ?? "light")
    });
globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
</script><script type="module">
let result;
try { const { runDesktopPerfFixture } = await import("/fixture.js"); result = { ok: true, tests: await runDesktopPerfFixture() }; }
catch (error) { result = { ok: false, error: String(error && error.stack || error) }; }
await fetch("/report/${token}", { method: "POST", body: JSON.stringify(result) });
</script>`,
  files,
);

interface ProfileNode {
  id: number;
  callFrame: { functionName: string; url: string; lineNumber: number };
  hitCount?: number;
}
function hottest(
  profile: { nodes: ProfileNode[]; samples: number[]; timeDeltas: number[] },
  top = 25,
) {
  const self = new Map<number, number>();
  profile.samples.forEach((id, index) =>
    self.set(id, (self.get(id) ?? 0) + (profile.timeDeltas[index] ?? 0))
  );
  const byFrame = new Map<string, number>();
  let total = 0;
  for (const node of profile.nodes) {
    const micros = self.get(node.id) ?? 0;
    total += micros;
    const frame = node.callFrame;
    const key = `${frame.functionName || "(anonymous)"} ${
      frame.url.split("/").pop()
    }:${frame.lineNumber + 1}`;
    byFrame.set(key, (byFrame.get(key) ?? 0) + micros);
  }
  return [...byFrame].sort((a, b) => b[1] - a[1]).slice(0, top)
    .map(([key, micros]) =>
      `${(micros / 1000).toFixed(1).padStart(8)}ms ${
        (100 * micros / total).toFixed(1).padStart(5)
      }%  ${key}`
    );
}

try {
  const mounted = await page.report;
  if (mounted?.ok !== true) {
    throw new Error(`fixture failed: ${JSON.stringify(mounted)}`);
  }
  await page.send("Profiler.enable");
  await page.send("Profiler.setSamplingInterval", { interval: 250 });
  const scenarios: Record<string, string> = {
    idle: "cowboyPerf.measure(() => new Promise((r) => setTimeout(r, 3000)))",
    churn: "cowboyPerf.measure(() => cowboyPerf.churn(5000, 250))",
    stream: "cowboyPerf.measure(() => cowboyPerf.stream(8000))",
    switch: "cowboyPerf.measure(() => cowboyPerf.switchSessions(8))",
  };
  for (const [name, expression] of Object.entries(scenarios)) {
    if (only && !only.includes(name)) continue;
    await page.send("Profiler.start");
    const measured = await page.evaluate<Record<string, unknown>>(expression);
    const { profile } = await page.send("Profiler.stop");
    await writeFile(join(out, `${name}.cpuprofile`), JSON.stringify(profile));
    console.log(`\n== ${name}`, JSON.stringify(measured));
    for (const line of hottest(profile)) console.log(line);
  }
  if (!only || only.includes("type")) {
    // Trusted keystrokes into the Prompt editor while busy sessions stream.
    const focused = await page.evaluate<string>(`(() => {
      const editor = document.querySelector("[contenteditable='true'], textarea");
      editor?.focus();
      const active = document.activeElement;
      return (editor ? "editor " : "no-editor ") + (active?.className ?? "") + " " + (localStorage.getItem("cowboy:vim") ?? "");
    })()`);
    console.log("focus", focused);
    // A realistic long prompt: headings, lists, inline code and a fence.
    const prompt = Array.from({ length: 40 }, (_, i) =>
      `## Step ${i}\n\n- check \`module_${i}\` and **why** it fails\n- then [link](https://x.invalid/${i})\n\n\`\`\`rust\nlet v${i} = run(${i})?;\n\`\`\`\n`
    ).join("\n");
    if (process.env.PERF_PROMPT !== "empty") {
      await page.send("Input.insertText", { text: prompt });
      await new Promise((resolve) => setTimeout(resolve, 1500));
    }
    await page.evaluate(`(() => {
      globalThis.__perfEvents = [];
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          if (["keydown", "keypress", "keyup", "beforeinput", "input"].includes(entry.name)) {
            globalThis.__perfEvents.push(entry.duration);
          }
        }
      }).observe({ type: "event", durationThreshold: 16 });
      return true;
    })()`);
    await page.send("Profiler.start");
    await page.evaluate(
      `(globalThis.__perfTyping = cowboyPerf.measure(() => cowboyPerf.stream(4000, { chunkMs: 60 })), true)`,
    );
    const typed =
      "Please look at the failing test and explain why the borrow outlives the scope. ";
    for (const character of typed) {
      await page.send("Input.dispatchKeyEvent", {
        type: "keyDown",
        key: character,
        text: character,
        unmodifiedText: character,
      });
      await page.send("Input.dispatchKeyEvent", {
        type: "keyUp",
        key: character,
      });
      await new Promise((resolve) => setTimeout(resolve, 35));
    }
    const measured = await page.evaluate<Record<string, unknown>>(
      "globalThis.__perfTyping",
    );
    const { profile } = await page.send("Profiler.stop");
    await writeFile(join(out, "type.cpuprofile"), JSON.stringify(profile));
    await new Promise((resolve) => setTimeout(resolve, 500));
    const events = await page.evaluate<number[]>("globalThis.__perfEvents");
    const sortedEvents = [...events].sort((a, b) => a - b);
    console.log("input events >=16ms", JSON.stringify({
      keys: typed.length,
      slow: events.length,
      p50: sortedEvents[Math.floor(sortedEvents.length / 2)] ?? 0,
      max: sortedEvents.at(-1) ?? 0,
    }));
    const echoed = await page.evaluate<string>(
      `(document.activeElement?.value ?? document.activeElement?.textContent)?.slice(0, 40) ?? ""`,
    );
    console.log("\n== type", JSON.stringify({ ...measured, echoed }));
    for (const line of hottest(profile)) console.log(line);
  }
  const shot = await page.send("Page.captureScreenshot", { format: "png" });
  await writeFile(join(out, "final.png"), Buffer.from(shot.data, "base64"));
  console.log(
    "\nfetches",
    JSON.stringify(
      await page.evaluate("Object.fromEntries(cowboyPerfFetches)"),
    ),
  );
} finally {
  await page.close();
}
