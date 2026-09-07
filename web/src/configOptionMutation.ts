import type { ConfigOption } from "./protocol";

export interface ConfigOptionChange {
  configId: string;
  value: string | boolean;
}

export function configOptionsMatchChanges(
  options: readonly ConfigOption[],
  changes: readonly ConfigOptionChange[],
): boolean {
  const current = new Map(
    options.map((option) => [option.id, option.currentValue]),
  );
  return changes.every((change) =>
    current.get(change.configId) === change.value
  );
}
