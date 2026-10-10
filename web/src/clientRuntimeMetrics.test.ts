import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import { localStorageMetrics } from "./clientRuntimeMetrics.ts";

function storage(entries: readonly (readonly [string, string])[]): Storage {
  const values = new Map(entries);
  return {
    get length() {
      return values.size;
    },
    key(index: number) {
      return [...values.keys()][index] ?? null;
    },
    getItem(key: string) {
      return values.get(key) ?? null;
    },
  } as Storage;
}

test("client local storage reports bytes, entries, and composer drafts", () => {
  assertEquals(
    localStorageMetrics(storage([
      ["theme", "dark"],
      ["cowboy:composer-draft:sess-1", '{"text":"你好"}'],
    ])),
    {
      bytes: new TextEncoder().encode(
        'themedarkcowboy:composer-draft:sess-1{"text":"你好"}',
      ).byteLength,
      entries: 2,
      drafts: 1,
    },
  );
});

test("client local storage degrades to an empty readable snapshot", () => {
  assertEquals(localStorageMetrics(undefined), {
    bytes: 0,
    entries: 0,
    drafts: 0,
  });
});

test("About storage distinguishes service and current-device metrics", async () => {
  const source = await readFile(
    new URL("./InfoSheet.tsx", import.meta.url), "utf8",
  );
  assert(source.includes('data-storage-scope="service"'));
  assert(source.includes('data-storage-scope="client"'));
  assert(source.includes("Shared Cowboy daemon and durable session store"));
  assert(source.includes("This browser or app on the current device"));
  assert(source.includes('"App storage used"'));
  assert(source.includes('"App storage allowance"'));
  assert(source.includes("`Up to ${formatBytes(metrics.storageQuotaBytes)}`"));
  assert(source.includes("heap === undefined ? []"));
  assert(source.includes('[["JS heap", heap] as const]'));
  assert(!source.includes('"Not exposed by browser"'));
  const infoStart = source.indexOf("export function InfoContent");
  const storageStart = source.indexOf("\n            Storage\n", infoStart);
  const usageStorageBoundary = source.slice(infoStart, storageStart);
  assertEquals(
    usageStorageBoundary.match(/\{!desktop && <Divider \/>\}/gu)?.length,
    1,
  );
});
