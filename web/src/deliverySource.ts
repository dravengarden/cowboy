// The delivery picker acts on the draft it was opened for. A choice made after
// the draft changed (typing, a new attachment, another session) must not send
// the stale snapshot; it closes the picker and returns to the editor instead.

export interface DeliverySource<Attachments> {
  session: string;
  text: string;
  /** Compared by identity: the composer replaces the array on every change. */
  attachments: Attachments;
}

export function deliverySourceMatches<Attachments>(
  source: DeliverySource<Attachments> | null | undefined,
  current: DeliverySource<Attachments>,
): boolean {
  return source?.session === current.session &&
    source.attachments === current.attachments &&
    source.text === current.text;
}

/** Wrap a picker action: a current draft dismisses the picker and runs the
 *  action; a stale one only closes the picker. */
export function guardDeliveryChoice<Args extends unknown[]>(
  handlers: { isCurrent: () => boolean; closeStale: () => void; dismiss: () => void },
  run: (...args: Args) => void,
): (...args: Args) => void {
  return (...args) => {
    if (!handlers.isCurrent()) return handlers.closeStale();
    handlers.dismiss();
    run(...args);
  };
}
