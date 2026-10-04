/** Bootstrap layout frames arrive in separate WebSocket tasks. Publishing the
 * session index before its title/order/folder overlays paints an intermediate
 * layout. Keep this buffer local to one socket; cached state and admission
 * controls remain usable while the new connection assembles its baseline. */
export function createBootstrapPresentation<T extends { readonly type: string }>(
  apply: (message: T) => void,
): (message: T) => void {
  let pending: T[] | undefined = [];
  return (message): void => {
    if (pending === undefined) {
      apply(message);
    } else if (message.type === "bootstrap_complete") {
      const baseline = pending;
      pending = undefined;
      for (const frame of baseline) apply(frame);
      apply(message);
    } else if (
      message.type === "sessions" || message.type === "machines" ||
      message.type === "sync_patch"
    ) {
      pending.push(message);
    } else {
      apply(message);
    }
  };
}
