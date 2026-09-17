/** Public synthetic identities only; never a production discovery or grant. */
import golden from "../../../contracts/code-buffer-sync.fixture.json" with {
  type: "json",
};
import { captureContent } from "./content.ts";
import { opened } from "./fixture.ts";
import type { SynchronizationState } from "./synchronizationProtocol.ts";

export { golden };
export const SYNC_ID = golden.operationId;
export const content = () => captureContent("abc");
export function syncWire(
  state: SynchronizationState = { kind: "prepared" },
  pending = false,
) {
  return { ...golden, state, pending };
}
export const appliedState: SynchronizationState = {
  kind: "applied",
  content: golden.content,
  version: golden.state.version,
};
export async function preparedSync() {
  const f = await opened();
  const captured = await content();
  const preparing = f.owner.prepareSynchronization(captured);
  f.reply(2, syncWire());
  const operation = await preparing;
  const source = f.registry.synchronizations;
  const row = source.get().rows[0]!;
  return { ...f, captured, operation, source, row };
}
