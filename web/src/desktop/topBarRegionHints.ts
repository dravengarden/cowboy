// The status line's Top bar legend (FOCUS.md "Commands and help"). The
// registry owns the keys, so a conditional action (Verify, Update) is listed
// exactly while its command can run and a disabled one drops out, instead of
// a hand-written list that advertises keys which do nothing.

export interface TopBarCommandLike {
  readonly id: string;
  readonly title: string;
  readonly shortcut?: string;
  readonly regions?: readonly string[];
  readonly when?: () => boolean;
}

export interface TopBarRegionHint {
  readonly keys: string;
  readonly label: string;
}

/** The verb the status line shows for each Top bar command. */
const TOP_BAR_HINT_LABELS: Readonly<Record<string, string>> = {
  "topbar.runConfiguration": "Config",
  "topbar.usage": "Usage",
  "topbar.reauthenticate": "Verify",
  "topbar.providerUpdate": "Update",
  "topbar.reload": "Reload",
  "topbar.compact": "Compact",
  "topbar.clear": "Clear",
};

/** Bare-key Top bar commands that can run now, in registration order. */
export function topBarRegionHints(
  commands: readonly TopBarCommandLike[],
): TopBarRegionHint[] {
  return commands.flatMap((command) => {
    const bare = command.shortcut !== undefined && !command.shortcut.includes("+");
    if (
      !bare || !command.regions?.includes("topbar.controls") ||
      command.when?.() === false
    ) return [];
    return [{
      keys: command.shortcut as string,
      label: TOP_BAR_HINT_LABELS[command.id] ?? command.title,
    }];
  });
}
