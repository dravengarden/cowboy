// Widget rows in the transcript (docs/widgets.md §6.2, §6.5). The Controller
// projects each widget as a `notice` carrying its snapshot under
// `_cowboy.widget`; an older client draws the notice text, this one the
// widget. Every later snapshot of the same widget replaces the row that the
// first one created, and only a newer `stateVersion` wins.

export type WidgetRowData = {
  widgetId: string;
  handle: string;
  revision: number;
  stateVersion: number;
  closed: boolean;
  /** When the widget last changed; absent from snapshots that predate it. */
  updatedAtMs?: number | undefined;
  document: unknown;
};

/** The snapshot a notice update carries, when it is a widget row. */
export function widgetOfNotice(update: Record<string, unknown>): WidgetRowData | undefined {
  const cowboy = update["_cowboy"];
  if (typeof cowboy !== "object" || cowboy === null) return undefined;
  const widget = (cowboy as Record<string, unknown>)["widget"];
  if (typeof widget !== "object" || widget === null) return undefined;
  const w = widget as Record<string, unknown>;
  if (typeof w["widgetId"] !== "string" || typeof w["stateVersion"] !== "number") return undefined;
  return {
    widgetId: w["widgetId"],
    handle: typeof w["handle"] === "string" ? w["handle"] : "",
    revision: typeof w["revision"] === "number" ? w["revision"] : 0,
    stateVersion: w["stateVersion"],
    closed: w["closed"] === true,
    ...(typeof w["updatedAtMs"] === "number" ? { updatedAtMs: w["updatedAtMs"] } : {}),
    document: w["document"],
  };
}

type NoticeLike = { kind: string; key: string; widget?: WidgetRowData | undefined };

/** Places a notice: a widget snapshot replaces the row its widget already has
 *  (keeping that row's key and position) when it is newer, and is dropped when
 *  it is not; any other notice is appended. */
export function placeNotice<T extends NoticeLike>(items: T[], notice: T): void {
  const widget = notice.widget;
  if (widget) {
    const index = items.findIndex(
      (item) => item.kind === "notice" && item.widget?.widgetId === widget.widgetId,
    );
    const existing = index < 0 ? undefined : items[index];
    if (existing) {
      if ((existing.widget?.stateVersion ?? 0) < widget.stateVersion)
        items[index] = { ...notice, key: existing.key };
      return;
    }
  }
  items.push(notice);
}
