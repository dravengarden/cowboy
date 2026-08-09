// Pure persisted-order model. Keep this module free of React/MUI imports so its
// migrations can run in the repository's capability-restricted Deno tests.

export const DEFAULT_COMPOSER_TOOLBAR: readonly string[] = [
  "undo",
  "redo",
  "bold",
  "italic",
  "code",
  "link",
  "heading",
  "bulletList",
  "numberedList",
  "checklist",
  "quote",
  "codeBlock",
  "highlight",
  "strikethrough",
  "indent",
  "outdent",
  "mention",
  "slash",
];

// Migrate only the exact retired default. A genuinely curated device order is
// user-owned and must remain untouched.
const LEGACY_COMPOSER_TOOLBAR: readonly string[] = [
  "undo",
  "redo",
  "heading",
  "bold",
  "italic",
  "strikethrough",
  "highlight",
  "code",
  "link",
  "bulletList",
  "numberedList",
  "checklist",
  "quote",
  "codeBlock",
  "indent",
  "outdent",
  "mention",
  "slash",
  "attach",
];

function sameToolbar(
  left: readonly string[],
  right: readonly string[],
): boolean {
  return left.length === right.length &&
    left.every((id, index) => id === right[index]);
}

export function normalizeComposerToolbarOrder(
  value: unknown,
  isKnown: (id: string) => boolean,
): string[] {
  if (!Array.isArray(value) || !value.every((id) => typeof id === "string")) {
    return [...DEFAULT_COMPOSER_TOOLBAR];
  }
  const ids = value.filter((id): id is string => isKnown(id));
  return sameToolbar(ids, LEGACY_COMPOSER_TOOLBAR)
    ? [...DEFAULT_COMPOSER_TOOLBAR]
    : ids;
}
