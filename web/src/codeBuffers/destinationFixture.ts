/** Public fake responses only; the matching Rust handler exercises serialization. */
import golden from "../../../contracts/code-buffer-destination.fixture.json" with {
  type: "json",
};
import { navigationWire, preparedNavigation } from "./navigationFixture.ts";
export { golden };
export function destinationWire(
  state: "prepared" | "pending" | "unknown" | "expired" = "prepared",
  pending = false,
) {
  return {
    ...golden,
    pending,
    destinations: [{
      ...golden.destinations[0]!,
      state,
      resourceId: state === "prepared"
        ? golden.destinations[0]!.resourceId
        : null,
    }],
  };
}
export async function retainedNavigation(locations = golden.locations) {
  const f = await preparedNavigation();
  const execute = f.operation.execute();
  f.reply(3, { ...navigationWire("retained"), locations });
  await execute;
  return { ...f, target: f.operation.targets()[0]! };
}
export async function handedOff() {
  const f = await retainedNavigation();
  const preparing = f.operation.prepareDestination(f.target);
  f.reply(4, golden);
  const child = await preparing;
  return { ...f, child };
}
export function targetText(resourceId = golden.destinations[0]!.resourceId) {
  return {
    apiVersion: 1,
    resourceId,
    openedVersion: [],
    result: {
      kind: "text",
      content: golden.locations[0]!.content,
      result: {
        kind: "page",
        snapshot: "a".repeat(64),
        offset: 0,
        text: "abc",
        nextOffset: null,
      },
    },
  };
}
