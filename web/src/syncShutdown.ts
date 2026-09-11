/** Local dependency ordering only, not a distributed effect/rollback executor.
 * Seal ALL writers synchronously, drain their final writes (including failures),
 * then release their borrowed database. Reentrant calls share the same barrier.
 */
interface DisposableSyncOwner {
  dispose(): Promise<void>;
}

export function createSyncShutdown(
  database: DisposableSyncOwner,
): (clients: readonly DisposableSyncOwner[]) => Promise<void> {
  let disposal: Promise<void> | undefined;
  return (clients): Promise<void> => {
    if (disposal) return disposal;
    const pending: Promise<void>[] = [];
    disposal = Promise.resolve().then(async () => {
      const outcomes = await Promise.allSettled(pending);
      const failures = outcomes.flatMap((outcome) =>
        outcome.status === "rejected" ? [outcome.reason] : []
      );
      try {
        await database.dispose();
      } catch (error) {
        failures.push(error);
      }
      if (failures.length) {
        throw new AggregateError(failures, "sync owner cleanup failed");
      }
    });
    for (const client of clients) {
      try {
        pending.push(client.dispose());
      } catch (error) {
        pending.push(Promise.reject(error));
      }
    }
    return disposal;
  };
}
