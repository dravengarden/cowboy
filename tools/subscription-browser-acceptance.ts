/** Real product surfaces, isolated HTTP fixtures and trusted Tab input. */
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { openFixturePage } from "./cdp-fixture.ts";

const [endpoint = "http://127.0.0.1:9222", destination, theme = "light"] =
  process.argv.slice(2);
if (!destination || !["light", "dark"].includes(theme)) {
  throw new Error(
    "usage: subscription-browser-acceptance.ts ENDPOINT OUTPUT [light|dark]",
  );
}
const output = resolve(destination);
await mkdir(output, { recursive: true });
const page = await openFixturePage(
  endpoint,
  "subscriptions",
  (token) =>
    `<!doctype html><meta name="viewport" content="width=device-width, initial-scale=1"><script>
localStorage.setItem("cowboy:theme-system-default-v1", "1");
localStorage.setItem("cowboy-theme-mode", ${JSON.stringify(theme)});
globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
globalThis.begin = new Promise(resolve => { globalThis.start = resolve; });
</script><script type="module">
let result;
try {
  const { runSubscriptionsBrowserConformance } = await import("/fixture.js");
  await globalThis.begin;
  result = { ok: true, tests: await runSubscriptionsBrowserConformance(async (surface, stage) => {
    globalThis.stage = surface + "-" + stage;
    await new Promise(resolve => { globalThis.next = resolve; });
  }) };
} catch (error) { result = { ok: false, error: String(error) }; }
await fetch("/report/${token}", { method:"POST", body:JSON.stringify(result) });
</script>`,
);
try {
  const resize = (mobile: boolean) =>
    page.send("Emulation.setDeviceMetricsOverride", {
      width: mobile ? 390 : 1280,
      height: mobile ? 844 : 900,
      deviceScaleFactor: 1,
      mobile,
    });
  await resize(true);
  const until = async (expression: string) => {
    const deadline = Date.now() + 30_000;
    while (!await page.evaluate<boolean>(expression)) {
      if (Date.now() > deadline) {
        throw new Error(`browser stage timed out: ${expression}`);
      }
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  await until("typeof globalThis.start === 'function'");
  await page.evaluate("globalThis.start()");
  const screenshots: string[] = [];
  for (const surface of ["mobile", "desktop"]) {
    await resize(surface === "mobile");
    for (
      const stage of [
        "accounts",
        "login",
        "cancelled",
        "revocation",
        "handoff",
        "verifying",
        "signed-in",
        "expired",
        "restart",
      ]
    ) {
      await until(
        `globalThis.stage === ${JSON.stringify(`${surface}-${stage}`)}`,
      );
      await new Promise((resolve) => setTimeout(resolve, 400));
      // Touch sheets intentionally preserve the existing responder. Establish
      // an explicit form focus before testing hardware-keyboard traversal.
      if (
        surface === "mobile" && (stage === "login" || stage === "revocation")
      ) {
        await page.evaluate(
          "document.querySelector('[role=dialog] button:not([disabled])')?.focus()",
        );
      }
      if (stage === "login" || stage === "revocation") {
        await page.send("Input.dispatchKeyEvent", {
          type: "keyDown",
          key: "Tab",
          code: "Tab",
          windowsVirtualKeyCode: 9,
        });
        await page.send("Input.dispatchKeyEvent", {
          type: "keyUp",
          key: "Tab",
          code: "Tab",
          windowsVirtualKeyCode: 9,
        });
        if (
          !await page.evaluate<boolean>(
            "document.activeElement !== document.body && Boolean(document.activeElement?.closest('[role=dialog]'))",
          )
        ) {
          throw new Error(`${surface} ${stage}: Tab escaped the dialog`);
        }
      }
      const screenshot = await page.send("Page.captureScreenshot", {
        format: "png",
      });
      const file = `${output}/${surface}-${stage}-${theme}.png`;
      await writeFile(file, Buffer.from(screenshot.data, "base64"));
      screenshots.push(file);
      await page.evaluate("globalThis.stage = null; globalThis.next()");
    }
  }
  const result = await page.report;
  if (!result.ok || result.tests?.length !== 108) {
    throw new Error(JSON.stringify(result));
  }
  console.log(
    JSON.stringify(
      {
        ok: true,
        browser: page.browser,
        theme,
        fixture_sha256: page.digest,
        tests: result.tests,
        screenshots,
        physical_device: "not_checked",
      },
      null,
      2,
    ),
  );
} finally {
  await page.close();
}
