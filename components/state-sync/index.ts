// Cowboy-owned pure-TypeScript local-first state-sync component.
//
// A central ARBITER owns the truth; clients apply deterministic MUTATORS
// locally for instant UI, send them to the arbiter, and REBASE their pending
// mutations onto each broadcast PATCH (Replicache model). Correctness lives in
// the client engine + the convergence fuzz; the arbiter is a thin serializer a
// Rust service can implement natively against the same `Mutation`/`Patch`
// contract. The Merkle-DAG "incremental diff" idea is deferred behind `Patch`.
//
// See packages/sync/README.md and the cowboy `state-sync-engine` task.

export { createClient } from "./src/client.ts";
export type { Client, ClientOpts } from "./src/client.ts";
export { createArbiter } from "./src/arbiter.ts";
export type { Arbiter, ArbiterOpts } from "./src/arbiter.ts";
export { replicatedStore } from "./src/replicated.ts";
export type { ReplicatedOpts, ReplicatedStore } from "./src/replicated.ts";
export { mirroredStore } from "./src/mirrored.ts";
export type { MirroredOpts, MirroredStore, SyncStatus } from "./src/mirrored.ts";
export { applyMutation } from "./src/mutators.ts";
export type { ArgsOf, Mutator, Mutators } from "./src/mutators.ts";
export { snapshotPatch } from "./src/patch/snapshot.ts";
export { hashValue } from "./src/hash.ts";
export type {
  ClientId,
  ClientSnapshot,
  CommitRecord,
  LocalPersistence,
  Mutation,
  MutationId,
  Patch,
  RemoteBackend,
  SyncState,
  Version,
} from "./src/types.ts";
