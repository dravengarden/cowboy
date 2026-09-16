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
  "sourceMode",
];

// Migrate only an exact retired default. A genuinely curated device order is
// user-owned and must remain untouched, so a list is retired here ONLY when it
// was itself a shipped default — never because it merely looks close to one.
const RETIRED_COMPOSER_TOOLBARS: readonly (readonly string[])[] = [[
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
], [
  // Retired when Source mode was added; identical to the current default
  // without its last entry.
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
]];

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
  return RETIRED_COMPOSER_TOOLBARS.some((retired) => sameToolbar(ids, retired))
    ? [...DEFAULT_COMPOSER_TOOLBAR]
    : ids;
}
