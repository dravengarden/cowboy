import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { DOM_APP_SANDBOX } from "./domAppHost";
import { domAppsEnabledFor } from "./domAppsEnabled";

// The shell shares Cowboy's URL origin: `allow-same-origin` on its frame
// would hand an agent-written app Cowboy's cookies, storage and API
// (docs/widget-apps-dom.md §3). The frame's sandbox is exactly this token.
describe("full-DOM app frame", () => {
  test("the sandbox is exactly allow-scripts", () => {
    expect(DOM_APP_SANDBOX).toBe("allow-scripts");
    const host = readFileSync(new URL("./domAppHost.ts", import.meta.url), "utf8");
    const assignments = host.match(/setAttribute\("sandbox", [^)]+\)/g) ?? [];
    expect(assignments).toEqual(['setAttribute("sandbox", DOM_APP_SANDBOX)']);
  });

  test("no DOM app source grants same-origin or other sandbox powers", () => {
    const dir = new URL("./", import.meta.url);
    for (const file of readdirSync(dir).filter((name) => !name.endsWith(".test.ts"))) {
      // Code, not prose: comments may name the tokens they forbid.
      const source = readFileSync(new URL(file, dir), "utf8")
        .replace(/\/\*[\s\S]*?\*\//g, "")
        .replace(/^\s*\/\/.*$/gm, "");
      for (const token of ["allow-same-origin", "allow-popups", "allow-top-navigation", "allow-forms", "allow-modals"]) {
        expect(source.includes(token), `${file} grants ${token}`).toBe(false);
      }
    }
  });

  test("admission follows the table and excludes shells", () => {
    const chrome = (version: string, platform = "macOS") => ({
      navigator: {
        userAgentData: { brands: [{ brand: "Google Chrome", version }], mobile: false, platform },
      } as unknown as Navigator,
    });
    expect(domAppsEnabledFor(chrome("157"))).toBe(true);
    expect(domAppsEnabledFor(chrome("156"))).toBe(false);
    expect(domAppsEnabledFor(chrome("157", "Linux"))).toBe(false);
    expect(domAppsEnabledFor({ ...chrome("157"), webkit: { messageHandlers: {} } })).toBe(false);
  });
});
