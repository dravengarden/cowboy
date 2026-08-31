import { assertEquals } from "jsr:@std/assert";
import {
  DEFAULT_PASSKEY_REAUTH_INTERVAL_MS,
  normalizePasskeyReauthInterval,
  PASSKEY_REAUTH_INTERVALS,
} from "./passkeyIntervals.ts";

const HOUR_MS = 60 * 60 * 1_000;
const DAY_MS = 24 * HOUR_MS;

Deno.test("Passkey verification intervals use the short closed schedule", () => {
  assertEquals(
    PASSKEY_REAUTH_INTERVALS.map((option) => option.value),
    [1, 2, 3, 4, 6, 12].map((hours) => hours * HOUR_MS).concat([
      DAY_MS,
      2 * DAY_MS,
      3 * DAY_MS,
    ]),
  );
  assertEquals(DEFAULT_PASSKEY_REAUTH_INTERVAL_MS, DAY_MS);
  assertEquals(
    PASSKEY_REAUTH_INTERVALS.find((option) => option.value === DAY_MS)?.label,
    "Every day · Default",
  );
});

Deno.test("retired Passkey intervals migrate to stricter supported values", () => {
  assertEquals(
    normalizePasskeyReauthInterval(8 * HOUR_MS, 3 * DAY_MS),
    6 * HOUR_MS,
  );
  assertEquals(
    normalizePasskeyReauthInterval(7 * DAY_MS, 3 * DAY_MS),
    DAY_MS,
  );
  assertEquals(
    normalizePasskeyReauthInterval(14 * DAY_MS, 3 * DAY_MS),
    DAY_MS,
  );
  assertEquals(
    normalizePasskeyReauthInterval(7 * DAY_MS, 4 * HOUR_MS),
    4 * HOUR_MS,
  );
});
