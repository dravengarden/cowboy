import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { fireLabel, fireRel, scheduleQuickPresets, toDtLocal } from "./scheduleTime.ts";

test("fireLabel and fireRel format English relative and calendar times", () => {
  const now = new Date(2026, 7, 20, 19, 15).getTime();
  assertEquals(fireLabel(new Date(2026, 7, 20, 20, 0).getTime(), now), "Today 20:00");
  assertEquals(fireLabel(new Date(2026, 7, 21, 9, 0).getTime(), now), "Tomorrow 9:00");
  assertEquals(fireRel(now + 20_000, now), "Soon");
  assertEquals(fireRel(now + 45 * 60_000, now), "in 45m");
  assertEquals(fireRel(now + 8 * 3_600_000, now), "in 8h");
});

test("scheduleQuickPresets returns future presets including Tomorrow 9:00", () => {
  const now = new Date(2026, 7, 20, 19, 15, 20).getTime();
  const presets = scheduleQuickPresets(now);
  assertEquals(presets.map((p) => p.id), ["15m", "1h", "3h", "tomorrow-9"]);
  assertEquals(presets.every((p) => p.fireAtMs > now), true);
  assertEquals(toDtLocal(presets[3]!.fireAtMs), "2026-08-21T09:00");
});

test("computeSchedulePreset computes fresh targets from activation time", () => {
  const t0 = new Date(2026, 7, 20, 19, 0, 0).getTime();
  const t1 = t0 + 10 * 60_000;
  assertEquals(scheduleQuickPresets(t1)[0]!.fireAtMs - t1, 15 * 60_000);
});
