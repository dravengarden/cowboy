export type PendingPanelDisclosureDecision =
  | "expand"
  | "collapse"
  | "discard-clean-edit-and-collapse"
  | "confirm-dirty-edit";

/** Prefer the mounted editor document. React's draft mirror can lag IME or a
 *  native-textarea keystroke, and treating that stale empty string as "cleared"
 *  deletes the row when the keyboard dismisses. */
export function pendingEditLiveText(
  editorValue: string | undefined,
  mirroredText: string,
): string {
  return editorValue !== undefined ? editorValue : mirroredText;
}

export function pendingRowVisibleText(
  messageText: string,
  committedText: string | null,
): string {
  return committedText ?? messageText;
}

/**
 * Resolve a Queue/Draft header disclosure without ever hiding a live edit.
 *
 * The persisted accordion preference and the edit transaction are separate
 * state machines. An active edit always wins visually: a dirty edit needs an
 * explicit resolution, while an untouched edit can be abandoned safely.
 */
export function pendingPanelDisclosureDecision({
  collapsed,
  editing,
  dirty,
}: {
  collapsed: boolean;
  editing: boolean;
  dirty: boolean;
}): PendingPanelDisclosureDecision {
  if (editing) {
    return dirty ? "confirm-dirty-edit" : "discard-clean-edit-and-collapse";
  }
  return collapsed ? "expand" : "collapse";
}
