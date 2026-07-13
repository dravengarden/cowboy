import { useSyncExternalStore } from "react";

export interface VimMacroRecording {
  register: string;
  stop: () => void;
}

let recording: VimMacroRecording | null = null;
const listeners = new Set<() => void>();

function publish(next: VimMacroRecording | null): void {
  if (
    recording?.register === next?.register && recording?.stop === next?.stop
  ) return;
  recording = next;
  for (const listener of listeners) listener();
}

export function setVimMacroRecording(register: string, stop: () => void): void {
  publish({ register, stop });
}

export function clearVimMacroRecording(register?: string): void {
  if (register !== undefined && recording?.register !== register) return;
  publish(null);
}

export function getVimMacroRecording(): VimMacroRecording | null {
  return recording;
}

export function useVimMacroRecording(): VimMacroRecording | null {
  return useSyncExternalStore(
    (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    () => recording,
    () => null,
  );
}
