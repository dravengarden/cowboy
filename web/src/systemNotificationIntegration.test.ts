import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";

const serviceWorker = await readFile(new URL("../public/sw.js", import.meta.url), "utf8");
const store = await readFile(new URL("./store.ts", import.meta.url), "utf8");
const settings = await readFile(new URL("./NotificationSettings.tsx", import.meta.url), "utf8");
const server = await readFile(new URL("../../src/server.rs", import.meta.url), "utf8");

test("service worker owns Apple push display and bounded session navigation", () => {
  assert(serviceWorker.includes('self.addEventListener("push"'));
  assert(serviceWorker.includes("validNotificationMessage(message)"));
  assert(serviceWorker.includes('self.addEventListener("notificationclick"'));
  assert(serviceWorker.includes("SAFE_SESSION_ID"));
  assert(serviceWorker.includes("client.navigate(target)"));
});

test("controller, not a visible page or Hermes, delivers session events", () => {
  assert(server.includes("run_web_push_notifications("));
  assert(server.includes("NotificationCategory::Permission"));
  assertEquals(server.includes("NotificationCategory::Completed"), false);
  assertEquals(server.includes("NotificationCategory::Input"), false);
  assert(server.includes("NotificationCategory::Error"));
  assertEquals(server.toLowerCase().includes("hermes"), false);
  assertEquals(store.includes("presentSessionNotification"), false);
});

test("mobile notification status clears the page divider and keeps its controls top-aligned", () => {
  assert(settings.includes('pt: { xs: 1.5, md: 0 }'));
  assert(settings.includes('alignItems="flex-start"'));
  assert(settings.includes("flexShrink: 0"));
});
