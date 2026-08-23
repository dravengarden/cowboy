import { persisted, type Store } from "@cowboy/state-store";

export type ComposerStackPanel = "plan" | "queued" | "draft";

const expandedPanel = persisted<ComposerStackPanel | null>(
  "cowboy:composer-stack-expanded",
  null,
  {
    serialize: (value) => value ?? "",
    deserialize: (raw): ComposerStackPanel | null => {
      if (raw === "plan" || raw === "queued" || raw === "draft") return raw;
      return null;
    },
  },
);

export function composerStackExpandedStore(): Store<ComposerStackPanel | null> {
  return expandedPanel;
}

/** Tap an expanded panel to collapse it; tap a collapsed one to expand it
 *  and collapse the other two. */
export function nextComposerStackExpanded(
  current: ComposerStackPanel | null,
  tapped: ComposerStackPanel,
): ComposerStackPanel | null {
  return current === tapped ? null : tapped;
}

export function toggleComposerStackPanel(panel: ComposerStackPanel): void {
  expandedPanel.set(nextComposerStackExpanded(expandedPanel.get(), panel));
}

export function expandComposerStackPanel(panel: ComposerStackPanel): void {
  expandedPanel.set(panel);
}

export function collapseComposerStackPanel(panel: ComposerStackPanel): void {
  if (expandedPanel.get() === panel) expandedPanel.set(null);
}
