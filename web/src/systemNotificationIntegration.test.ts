import { assert, assertEquals } from "jsr:@std/assert";

const serviceWorker = await Deno.readTextFile(new URL("../public/sw.js", import.meta.url));
const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
const server = await Deno.readTextFile(new URL("../../src/server.rs", import.meta.url));

Deno.test("service worker owns Apple push display and bounded session navigation", () => {
  assert(serviceWorker.includes('self.addEventListener("push"'));
  assert(serviceWorker.includes("validNotificationMessage(message)"));
  assert(serviceWorker.includes('self.addEventListener("notificationclick"'));
  assert(serviceWorker.includes("SAFE_SESSION_ID"));
  assert(serviceWorker.includes("client.navigate(target)"));
});

Deno.test("controller, not a visible page or Hermes, delivers session events", () => {
  assert(server.includes("run_web_push_notifications("));
  assert(server.includes("NotificationCategory::Permission"));
  assert(server.includes("NotificationCategory::Completed"));
  assert(server.includes("NotificationCategory::Input"));
  assert(server.includes("NotificationCategory::Error"));
  assertEquals(server.toLowerCase().includes("hermes"), false);
  assertEquals(store.includes("presentSessionNotification"), false);
});
