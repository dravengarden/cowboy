import {
  createIdbPersistenceOwner,
  type IdbFailureCode,
} from "@cowboy/state-sync-idb";

// Compile-only negative contracts; never imported by the application.
export function verifyIdbOwnerContracts(): void {
  const database = createIdbPersistenceOwner({ factory: null });
  const record = database.persistence<{ count: number }>("counter", {
    strictWrites: true,
  });
  void record.save({ count: 1 });
  const outbox = database.outbox<number>("queue");
  void outbox.save({ base: { version: 0, value: 1 }, pending: [] });
  // @ts-expect-error Outboxes persist a typed client snapshot, not raw values.
  void outbox.save(1);
  // @ts-expect-error Durable outbox writes cannot silently ignore errors.
  database.outbox<number>("other", { strictWrites: false });
  // @ts-expect-error A borrowed outbox cannot close the shared database.
  void outbox.dispose();
  // @ts-expect-error A borrowed store cannot close its provider.
  void record.dispose();
  // @ts-expect-error Record shape is enforced at the write boundary.
  void record.save({ count: "not a number" });
  // @ts-expect-error Data-only options cannot acquire a different database.
  database.persistence("key", { dbName: "other" });
  // @ts-expect-error Lifecycle state is readonly diagnostic evidence.
  database.lifecycle.phase = "disposed";
  // @ts-expect-error Failure codes are closed; native exception text is not one.
  const error: IdbFailureCode = "private native exception";
  void error;
  void database.dispose();
}
