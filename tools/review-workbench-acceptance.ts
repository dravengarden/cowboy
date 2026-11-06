/** Render the actual Review/store in anonymous Hawk Chrome, then exercise the
 * bottom workbench with trusted clicks. No product endpoints or credentials. */
import { mkdir, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { type FixturePage, openFixturePage } from "./cdp-fixture.ts";

const output = process.argv[2] ?? ".review-assets";
const theme = process.argv[3] ?? "light";
await mkdir(output, { recursive: true });
await rm(join(output, `${theme}-failure.png`), { force: true });
const page = await openFixturePage(
  "http://127.0.0.1:9222",
  "review-context",
  () =>
    `<!doctype html>
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover, interactive-widget=resizes-content">
<script>localStorage.setItem("cowboy:theme-system-default-v1","1");localStorage.setItem("cowboy-theme-mode",${JSON.stringify(
      theme,
    )});
globalThis.CowboyDeviceProof={proof:async()=>"fixture",resetChallenge(){},install(){}};</script>
<script type="module">const m=await import('/fixture.js');globalThis.preview=m.renderReviewContextPreview;</script>`,
);
const measurements: unknown[] = [];
async function until(expression: string) {
  for (let i = 0; i < 250; i++) {
    if (await page.evaluate(expression)) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`Timed out: ${expression}`);
}
async function click(selector: string) {
  const point = await page.evaluate<{ x: number; y: number }>(`(() => {
    const node=document.querySelector(${JSON.stringify(selector)});
    if(!node) throw new Error('Missing '+${JSON.stringify(selector)});
    const r=node.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};
  })()`);
  await page.send("Input.dispatchMouseEvent", {
    type: "mousePressed",
    button: "left",
    clickCount: 1,
    ...point,
  });
  await page.send("Input.dispatchMouseEvent", {
    type: "mouseReleased",
    button: "left",
    clickCount: 1,
    ...point,
  });
  await new Promise((resolve) => setTimeout(resolve, 350));
}
async function capture(name: string, selectors: string[], sideInset = 0) {
  const measure = await page.evaluate(`(() => {
    const selectors=${JSON.stringify(selectors)};
    const controls=selectors.map(selector=>{
      const n=document.querySelector(selector);if(!n)throw new Error('Missing '+selector);
      const r=n.getBoundingClientRect();
      if(r.top<innerHeight/2 || r.bottom>innerHeight+1 || r.left<${sideInset} || r.right>innerWidth-${sideInset}+1) throw new Error('Unreachable '+selector+' '+JSON.stringify(r));
      if(n.matches('button') && (r.height<44 || r.width<44))throw new Error('Small target '+selector+' '+JSON.stringify(r));
      return {selector,x:r.x,y:r.y,width:r.width,height:r.height};
    });
    if(document.documentElement.scrollWidth>innerWidth)throw new Error('Page overflow');
    const reading=${/^(document|diff)(-|$)/.test(name)} ? document.querySelector('[data-review-document-content]')?.getBoundingClientRect() : undefined;
    if(reading && reading.height<180)throw new Error('Reading area too short: '+reading.height);
    if(reading && (reading.left<${sideInset} || reading.right>innerWidth-${sideInset}+1))throw new Error('Reading content crosses the safe-area edge');
    return {width:innerWidth,height:innerHeight,controls,readingHeight:reading?.height};
  })()`);
  measurements.push({ name, theme, measure });
  const shot = await page.send("Page.captureScreenshot", { format: "png" });
  await writeFile(join(output, `${theme}-${name}.png`), Buffer.from(shot.data, "base64"));
}
async function viewport(width: number, height: number, mobile = true) {
  await page.send("Emulation.setDeviceMetricsOverride", {
    width,
    height,
    deviceScaleFactor: 1,
    mobile,
  });
  await page.send("Emulation.setTouchEmulationEnabled", {
    enabled: mobile,
    maxTouchPoints: mobile ? 5 : 1,
  });
  await new Promise((resolve) => setTimeout(resolve, 150));
}
async function readingSizes(name: string, controls: string[]) {
  await capture(name, controls);
  await viewport(320, 568);
  await capture(`${name}-narrow`, controls);
  await viewport(844, 390);
  await capture(`${name}-landscape`, controls);
  await page.send("Emulation.setSafeAreaInsetsOverride", {
    insets: { left: 44, right: 44, bottom: 21 },
  });
  await capture(`${name}-landscape-safe-area`, controls, 44);
  await viewport(390, 844);
  await page.send("Emulation.setSafeAreaInsetsOverride", { insets: { bottom: 34 } });
  await capture(`${name}-safe-area`, controls);
  await page.send("Emulation.setSafeAreaInsetsOverride", { insets: {} });
}
async function navigationActions() {
  await click('[aria-label="More review actions"]');
  await click('[aria-label="Open extensions"]');
  await until(
    "!!document.querySelector('[data-detent-sheet]') && !document.querySelector('[aria-label=\"Close review actions\"]')",
  );
  await capture("extensions", ['[data-detent-sheet] [aria-label="Close"]']);
  await click('[data-detent-sheet] [aria-label="Close"]');
  await until("!document.querySelector('[data-detent-sheet]')");
  await page.evaluate("fixture.select('project')");
  await until(
    "document.querySelector('[data-review-context-button]')?.textContent.includes('Suger project')",
  );
  await click('[aria-label="More review actions"]');
  await click('[aria-label="Back to session Improve review"]');
  await until(
    "document.querySelector('[data-review-context-button]')?.textContent.includes('Improve review') && !document.querySelector('[aria-label=\"Close review actions\"]')",
  );
}
async function run(p: FixturePage) {
  await viewport(390, 844);
  await until("typeof preview==='function'");
  await p.evaluate("void(globalThis.fixture=preview(true,false,true,false,false,true))");
  await until(
    "document.querySelector('[data-review-unread]')?.getAttribute('aria-label')==='51 unread'",
  );
  const controls = [
    "[data-review-compare]",
    "[data-review-unread]",
    "[data-mobile-repository-tabs]",
    "[data-review-context-button]",
    '[aria-label="Git changes"]',
  ];
  await capture("changes", controls);
  await click('[aria-label="More review actions"]');
  await click('[aria-label="Repository details"]');
  await until(
    "document.querySelector('[data-repository-project-path]')?.textContent==='/tasks/cowboy/feature' && !document.querySelector('[aria-label=\"Close review actions\"]')",
  );
  await capture("repository-details", ['[aria-label="Close repository details"]']);
  await click('[aria-label="Close repository details"]');
  await click("[data-review-compare]");
  await click('[aria-label="Other branches"] [role="button"]:last-child');
  await until(
    "document.querySelector('[data-review-compare]')?.getAttribute('aria-label')?.includes('long-review-comparison-name')",
  );
  await capture("long-branch", controls);
  await viewport(320, 568);
  await capture("long-branch-narrow", controls);
  await viewport(390, 844);
  await p.evaluate("document.documentElement.style.fontSize='20px'");
  await capture("large-text", controls);
  await p.evaluate("document.documentElement.style.removeProperty('font-size')");
  await click("[data-review-compare]");
  await click('[aria-label="Comparison scenarios"] [role="button"]:last-child');
  await until(
    "document.querySelector('[data-review-compare]')?.getAttribute('aria-label')?.includes('Ready to merge · origin/main')",
  );
  await p.evaluate(
    `(()=>{const n=document.querySelector('[data-review-navigation-inline] [data-mobile-overflow-layer]');n.scrollTop=n.scrollHeight})()`,
  );
  await capture("changes-end", controls);
  await click('[data-segmented-tab="history"]');
  await until("document.body.textContent.includes('Fixture')");
  await capture("history", controls.slice(2));
  await click('[data-segmented-tab="worktrees"]');
  await capture("worktrees", controls.slice(2));
  await click('[data-segmented-tab="changes"]');
  await viewport(320, 568);
  await capture("narrow", controls);
  await viewport(844, 390);
  await capture("landscape", controls);
  await viewport(390, 844);
  await p.evaluate("fixture.select('project')");
  await until("document.body.textContent.includes('No changes from this branch')");
  await capture(
    "empty",
    controls.filter((selector) => selector !== "[data-review-unread]"),
  );
  await p.evaluate(
    `(()=>{const r=document.querySelector('[data-review-repository-controls]').getBoundingClientRect();if(r.height>52)throw new Error('Empty review uses more than one repository toolbar row')})()`,
  );
  await viewport(320, 568);
  await capture(
    "empty-narrow",
    controls.filter((selector) => selector !== "[data-review-unread]"),
  );
  await p.evaluate(
    `(()=>{const r=document.querySelector('[data-review-repository-controls]').getBoundingClientRect();if(r.height>52)throw new Error('Empty narrow review should fit one repository row')})()`,
  );
  await viewport(390, 844);
  await p.evaluate("fixture.select('worktree')");
  await until("!!document.querySelector('[data-review-unread]')");
  await click('[aria-label="Worktree files"]');
  await until("!!document.querySelector('[data-code-tree-path=\"README.md\"]')");
  await capture("files", ["[data-review-find-bar]", "[data-review-context-button]"]);
  await click('[aria-label="More review actions"]');
  await until("!!document.querySelector('[aria-label=\"Refresh files\"]')");
  await capture("more-actions", [
    '[aria-label="Refresh files"]',
    '[aria-label="Close review actions"]',
  ]);
  await click('[aria-label="Expand folders"]');
  await click('[aria-label="Refresh files"]');
  await click('[aria-label="Close review actions"]');
  await viewport(390, 430);
  await click("input");
  await capture("keyboard-sized", ["[data-review-find-bar]", "[data-review-context-button]"]);
  await viewport(390, 844);
  await click('[data-code-tree-path="README.md"]');
  await until("!!document.querySelector('[data-markdown-review-preview]')");
  await readingSizes("document", [
    '[aria-label="Show Markdown source"]',
    '[aria-label="Open file outline"]',
    '[aria-label="Open file tree sidebar"]',
    "[data-review-context-button]",
  ]);
  await click('[aria-label="Open file tree sidebar"]');
  await capture("sidebar", ["[data-review-find-bar]", '[aria-label="Close worktree"]']);
  await click('[aria-label="Close worktree"]');
  await until("!document.querySelector('[data-mobile-drawer-presented=\"true\"]')");
  await new Promise((resolve) => setTimeout(resolve, 500));
  await click('[aria-label="Git changes"]');
  await until("!!document.querySelector('[data-review-compare]')");
  await p.evaluate(
    `(()=>{const n=document.querySelector('[data-review-navigation-inline] [data-mobile-overflow-layer]');n.scrollTop=0})()`,
  );
  await p.evaluate(
    `Array.from(document.querySelectorAll('[role="button"]')).find(n=>n.textContent?.startsWith('app.ts'))?.click()`,
  );
  await until("!!document.querySelector('[aria-label=\"Mark reviewed\"]')");
  await until("!!document.querySelector('[data-review-check-action=\"lint\"]')");
  await readingSizes("diff", [
    '[data-review-check-action="lint"]',
    '[aria-label="Next hunk"]',
    '[aria-label="Mark reviewed"]',
    '[aria-label="Next change"]',
    '[aria-label="Open Git changes sidebar"]',
  ]);
  await click('[data-review-check-action="lint"]');
  await until("fixture.requests.some(r=>r.startsWith('POST ') && r.endsWith('/checks'))");
  await until("!!document.querySelector('[data-review-check-status]')");
  await capture("diff-check-completed", ['[data-review-check-action="lint"]']);
  await click('[aria-label="Mark reviewed"]');
  await until("!!document.querySelector('[aria-label=\"Mark unreviewed\"]')");
  await click('[aria-label="Next change"]');
  await capture("diff-next", ['[aria-label="Previous change"]', '[aria-label="Next change"]']);
  await navigationActions();
  await viewport(1280, 900, false);
  await p.evaluate(
    "fixture.cleanup(); void(globalThis.fixture=preview(true,false,true,false,true))",
  );
  await until("!!document.querySelector('[aria-label=\"Refresh repository\"]')");
  await capture("desktop", ["[data-review-workbench]"]);
  await viewport(600, 900, false);
  await capture("desktop-narrow", ["[data-review-workbench]"]);
  await writeFile(
    join(output, `${theme}-design.html`),
    await p.evaluate<string>("document.documentElement.outerHTML"),
  );
}
try {
  await run(page);
  await writeFile(join(output, `${theme}-measure.json`), JSON.stringify(measurements, null, 2));
  console.log(
    JSON.stringify({
      ok: true,
      theme,
      screenshots: measurements.length,
      browser: page.browser,
      output,
    }),
  );
} catch (error) {
  const shot = await page.send("Page.captureScreenshot", { format: "png" });
  await writeFile(join(output, `${theme}-failure.png`), Buffer.from(shot.data, "base64"));
  throw error;
} finally {
  await page.close();
}
