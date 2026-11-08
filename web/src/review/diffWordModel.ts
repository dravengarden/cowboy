export interface WordChange {
  removedFrom: number;
  removedTo: number;
  addedFrom: number;
  addedTo: number;
}

const MAX_WORD_DIFF_LENGTH = 4_000;

export function changedWordRange(
  removed: string,
  added: string,
): WordChange | undefined {
  if (
    removed === added ||
    removed.length + added.length > MAX_WORD_DIFF_LENGTH
  ) {
    return undefined;
  }
  let prefix = 0;
  const prefixLimit = Math.min(removed.length, added.length);
  while (prefix < prefixLimit && removed[prefix] === added[prefix]) prefix += 1;

  let suffix = 0;
  while (
    suffix < removed.length - prefix &&
    suffix < added.length - prefix &&
    removed[removed.length - suffix - 1] === added[added.length - suffix - 1]
  ) {
    suffix += 1;
  }
  return {
    removedFrom: prefix,
    removedTo: removed.length - suffix,
    addedFrom: prefix,
    addedTo: added.length - suffix,
  };
}
