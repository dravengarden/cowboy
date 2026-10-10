import { readdirSync, readFileSync } from "node:fs";
export type TestHostPlugin = { id: string } & Record<string, unknown>;

/** Load package-authored host data for tests that exercise first-party behavior.
 * Production receives the equivalent validated inventory from /api/plugins. */
export function testFirstPartyHostPlugins(): TestHostPlugin[] {
  const root = new URL("../../plugins/", import.meta.url);
  return [...readdirSync(root, { withFileTypes: true })]
    .filter((entry) => entry.isDirectory())
    .sort((left, right) => left.name.localeCompare(right.name))
    .flatMap((entry) => {
      try {
        const host = JSON.parse(
          readFileSync(new URL(`${entry.name}/host.json`, root), "utf8"),
        ) as Record<string, unknown>;
        return [{ id: entry.name, ...host }];
      } catch (reason) {
        if ((reason as { code?: string }).code === "ENOENT") return [];
        throw reason;
      }
    });
}
