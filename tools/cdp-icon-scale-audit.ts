/** Report every visible icon or glyph in the integrated Desktop App that does
 * not follow the global font size, in a running Chrome. Unlike the control
 * audit in the fixture (each control's first svg), this measures every svg,
 * img and text glyph, including non-interactive status marks, and writes
 * screenshots at 100% and 150% for review.
 *
 * Usage: bun tools/cdp-icon-scale-audit.ts http://127.0.0.1:9222 <out-dir> [light|dark]
 */
import { mkdir, writeFile } from "node:fs/promises";
import { openFixturePage } from "./cdp-fixture.ts";

const [endpoint = "", output, theme = "light"] = process.argv.slice(2);
if (!output?.startsWith("/")) throw new Error("expected an absolute output directory");
await mkdir(output, { recursive: true });
const page = await openFixturePage(
  endpoint,
  "draft-documents",
  () =>
    `<!doctype html><script>
globalThis.CowboyDeviceProof = { proof: async () => "fixture", resetChallenge() {}, install() {} };
globalThis.__cowboyShortcutAudit = true;
</script><script type="module">
const { runDraftDocumentsBrowserConformance } = await import("/fixture.js");
runDraftDocumentsBrowserConformance().catch((error) => { globalThis.__cowboyShortcutAuditResult = ["error: " + error]; });
</script>`,
);
await page.send("Emulation.setEmulatedMedia", {
  features: [{ name: "prefers-color-scheme", value: theme }],
});
await page.send("Emulation.setDeviceMetricsOverride", {
  width: 1440, height: 900, deviceScaleFactor: 2, mobile: false,
});

const measure = `(() => {
  const visible = (element) => element.getClientRects().length > 0 &&
    !element.closest(".cm-editor") && getComputedStyle(element).visibility !== "hidden";
  const name = (element) => {
    const region = element.closest("[data-desktop-region]")?.dataset.desktopRegion ??
      element.closest("[data-desktop-pane]")?.dataset.desktopPane ?? "chrome";
    const owner = element.closest("[aria-label]")?.getAttribute("aria-label") ??
      element.closest("button,[role=button],[role=tab],[role=menuitem],a")?.textContent ?? "";
    const tag = element.tagName.toLowerCase();
    const cls = (element.getAttribute("class") ?? "").split(/\\s+/).find((c) =>
      /^(Mui[A-Za-z]+-root|css-)/.test(c) === false && c.length > 0) ?? "";
    const data = [...element.attributes].filter((a) => a.name.startsWith("data-")).map((a) => a.name).join(",");
    return (region + " :: " + owner.replace(/\\s+/g, " ").trim().slice(0, 40) + " :: " + tag +
      (data ? "[" + data + "]" : "") + (cls ? "." + cls : "")).slice(0, 160);
  };
  const items = [];
  for (const element of document.querySelectorAll("svg, img")) {
    if (element.tagName.toLowerCase() === "svg" && element.parentElement?.closest("svg")) continue;
    if (!visible(element)) continue;
    const box = element.getBoundingClientRect();
    if (box.width === 0 || box.height === 0) continue;
    items.push({ kind: "graphic", key: name(element), size: box.height });
  }
  for (const element of document.querySelectorAll("body *")) {
    if (!visible(element) || element.children.length > 0) continue;
    const text = element.textContent?.trim() ?? "";
    if (!text || element.closest("svg")) continue;
    items.push({ kind: text.length <= 2 ? "glyph" : "text", key: name(element) + " :: " + text.slice(0, 12),
      size: parseFloat(getComputedStyle(element).fontSize) });
  }
  return items;
})()`;

const scaleTo = async (scale: number) => {
  await page.evaluate(`(() => {
    document.documentElement.style.fontSize = "${scale * 100}%";
    document.documentElement.style.setProperty("--cowboy-font-scale", "${scale}");
  })()`);
  await new Promise((resolve) => setTimeout(resolve, 400));
};
const shot = async (file: string) => {
  const { data } = await page.send("Page.captureScreenshot", { format: "png" });
  await writeFile(`${output}/${file}`, Uint8Array.from(atob(data), (c) => c.charCodeAt(0)));
};

try {
  let held: unknown = null;
  for (let attempt = 0; attempt < 600 && !held; attempt++) {
    held = await page.evaluate("globalThis.__cowboyShortcutAuditResult ?? null");
    if (!held) await new Promise((resolve) => setTimeout(resolve, 300));
  }
  if (!held) throw new Error("the integrated App was not reached");
  await scaleTo(1);
  await shot(`${theme}-100.png`);
  const base = await page.evaluate<{ kind: string; key: string; size: number }[]>(measure);
  await scaleTo(1.5);
  await shot(`${theme}-150.png`);
  const large = await page.evaluate<{ kind: string; key: string; size: number }[]>(measure);
  await scaleTo(0.75);
  await shot(`${theme}-075.png`);
  const fixed: Record<string, Set<string>> = { graphic: new Set(), glyph: new Set(), text: new Set() };
  base.forEach((item, index) => {
    const after = large[index];
    if (!after || after.key !== item.key || item.size === 0) return;
    if (after.size / item.size < 1.2) fixed[item.kind]!.add(`${item.key} (${item.size}px)`);
  });
  const report = Object.fromEntries(Object.entries(fixed).map(([kind, set]) => [kind, [...set].sort()]));
  await writeFile(`${output}/${theme}-report.json`, JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ measured: base.length, graphic: report.graphic!.length, glyph: report.glyph!.length,
    text: report.text!.length }, null, 2));
} finally {
  await page.close();
}
