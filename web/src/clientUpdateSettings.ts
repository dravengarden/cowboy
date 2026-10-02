import { persisted, useStore } from "@cowboy/state-store";

export type ClientUpdateMode = "automatic" | "manual";
export function updateDelayFromStorage(raw: string): number {
  const value = Number(raw);
  return raw.trim() !== "" && Number.isInteger(value) && value >= 0 && value <= 3600 ? value : 3;
}
const mode = persisted<ClientUpdateMode>("cowboy:client-update-mode", "automatic", {
  serialize: String,
  deserialize: (raw) => raw === "manual" ? "manual" : "automatic",
});
const delay = persisted("cowboy:client-update-delay-seconds", 3, {
  serialize: String,
  deserialize: updateDelayFromStorage,
});
export function useClientUpdateSettings() {
  return { mode: useStore(mode), countdownSecs: useStore(delay) };
}
export function setClientUpdateMode(value: ClientUpdateMode): void { mode.set(value); }
export function setClientUpdateDelay(value: number): void { delay.set(updateDelayFromStorage(String(value))); }
