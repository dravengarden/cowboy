export interface VimEscapeState {
  insertMode?: boolean;
  visualMode?: boolean;
  inputState?: {
    operator?: unknown;
    keyBuffer?: string | readonly string[];
  };
}

function hasPendingKeys(keys: string | readonly string[] | undefined): boolean {
  return (keys?.length ?? 0) > 0;
}

/**
 * Vim owns Escape until the editor is in plain Normal mode.
 *
 * Insert, Visual, and operator/key-prefix states must first normalize through
 * Vim. Only a later Escape from plain Normal may reach Cowboy's active
 * surrounding transaction (discard edit or close layer). It never arms
 * workspace navigation. With Vim disabled, Cowboy
 * retains the established direct Escape behavior.
 */
export function vimEscapeBelongsToApp(
  vimEnabled: boolean,
  state: VimEscapeState | null | undefined,
): boolean {
  if (!vimEnabled) return true;
  if (!state) return false;
  return !state.insertMode &&
    !state.visualMode &&
    !state.inputState?.operator &&
    !hasPendingKeys(state.inputState?.keyBuffer);
}
