import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { orderedItems } from "./aboutPreferences";

test("layout order survives renamed/offline items and appends newly available items", () => {
  const items = [{ id: "hawk", name: "Renamed", online: false }, { id: "ovh", name: "OVH", online: true }, { id: "new", name: "New", online: true }];
  assertEquals(orderedItems(items, ["gone", "ovh", "hawk"], (item) => item.id).map((item) => item.id), ["ovh", "hawk", "new"]);
  assertEquals(orderedItems(items, [], (item) => item.id), items);
  assertEquals(items.map((item) => item.id), ["hawk", "ovh", "new"]);
});
