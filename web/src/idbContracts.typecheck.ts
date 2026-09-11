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
