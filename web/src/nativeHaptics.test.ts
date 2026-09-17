import { assertEquals } from "jsr:@std/assert";
import {
  haptic,
  notificationHaptic,
  selectionHaptic,
} from "../../components/app-shell/haptics.ts";

const pause = (): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, 60));

Deno.test("a shell haptic bridge replaces the Tauri plugin waveforms", async () => {
  const root = globalThis as Record<string, unknown>;
  const kinds: string[] = [];
  const invoked: string[] = [];
  root.__cowboyNativeHaptic = (kind: string) => {
    kinds.push(kind);
    return true;
  };
  root.__TAURI_INTERNALS__ = {
    invoke: (command: string) => {
      invoked.push(command);
      return Promise.resolve();
    },
  };
  try {
    haptic("light");
    await pause();
    notificationHaptic("error");
    await pause();
    selectionHaptic();
    assertEquals(kinds, ["impact:light", "notification:error", "selection"]);
    assertEquals(invoked, []);
  } finally {
    delete root.__cowboyNativeHaptic;
    delete root.__TAURI_INTERNALS__;
  }
});

Deno.test("a declined shell haptic falls back to the Tauri plugin", async () => {
  const root = globalThis as Record<string, unknown>;
  const invoked: string[] = [];
  await pause();
  root.__cowboyNativeHaptic = () => false;
  root.__TAURI_INTERNALS__ = {
    invoke: (command: string) => {
      invoked.push(command);
      return Promise.resolve();
    },
  };
  try {
    haptic("medium");
    assertEquals(invoked, ["plugin:haptics|impact_feedback"]);
  } finally {
    delete root.__cowboyNativeHaptic;
    delete root.__TAURI_INTERNALS__;
  }
});
