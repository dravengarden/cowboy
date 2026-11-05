import { readFile, writeFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { inboundFixtures } from "./protocol.inbound.fixtures.ts";

// contracts/protocol/inbound.json is what src/protocol_contract.rs parses.
// Regenerate with:
//   COWBOY_UPDATE_CONTRACTS=1 bun test src/protocol.inbound.fixtures.test.ts
const contract = new URL(
  "../../contracts/protocol/inbound.json",
  import.meta.url,
);

test("the Controller's Inbound contract matches the web fixtures", async () => {
  const fixtures = Object.entries(inboundFixtures).map(([name, value]) => ({
    name,
    value,
  }));
  const rendered = JSON.stringify(fixtures, null, 2) + "\n";
  if (process.env.COWBOY_UPDATE_CONTRACTS) await writeFile(contract, rendered);
  assertEquals(await readFile(contract, "utf8"), rendered);
});
