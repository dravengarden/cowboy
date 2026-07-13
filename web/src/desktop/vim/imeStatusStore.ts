import { useSyncExternalStore } from "react";

export type ImePhase = "idle" | "composing" | "committed";
export interface ImeStatus {
  phase: ImePhase;
  autoInserted: boolean;
}

const IDLE: ImeStatus = { phase: "idle", autoInserted: false };
let status = IDLE;
let settleTimer: number | null = null;
const listeners = new Set<() => void>();

function publish(next: ImeStatus): void {
  if (next.phase === status.phase && next.autoInserted === status.autoInserted) return;
  status = next;
  for (const listener of listeners) listener();
}

function clearSettleTimer(): void {
  if (settleTimer === null) return;
  globalThis.clearTimeout(settleTimer);
  settleTimer = null;
}

export function setImeComposing(autoInserted: boolean): void {
  clearSettleTimer();
  publish({ phase: "composing", autoInserted });
}

export function setImeCommitted(): void {
  clearSettleTimer();
  publish({ phase: "committed", autoInserted: false });
  settleTimer = globalThis.setTimeout(() => {
    settleTimer = null;
    publish(IDLE);
  }, 600);
}

export function clearImeStatus(): void {
  clearSettleTimer();
  publish(IDLE);
}

export function isImeComposing(): boolean {
  return status.phase === "composing";
}

export function useImeStatus(): ImeStatus {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => status,
    () => IDLE,
  );
}
