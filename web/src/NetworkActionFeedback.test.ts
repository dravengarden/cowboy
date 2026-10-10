import { test } from "bun:test";
import { assert } from "@std/assert";
import {
  NETWORK_PRESS_MIN_MS,
  NETWORK_PROGRESS_DELAY_MS,
  NETWORK_PROGRESS_MIN_MS,
} from "./networkActionPolicy";

test("network actions preserve fast-path feedback without spinner flash", () => {
  assert(NETWORK_PRESS_MIN_MS > 0);
  assert(NETWORK_PRESS_MIN_MS < NETWORK_PROGRESS_DELAY_MS);
  assert(NETWORK_PROGRESS_MIN_MS >= NETWORK_PROGRESS_DELAY_MS);
});
