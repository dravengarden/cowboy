/** Shared Rust/TypeScript wire evidence, no product endpoint or credentials. */
import golden from "../../../contracts/code-buffer-navigation.fixture.json" with {
  type: "json",
};
import { captureContent } from "./content.ts";
import { opened } from "./fixture.ts";
import type { NavigationState } from "./navigationProtocol.ts";

export { golden };
export const NAV_ID = golden.navigationId;
export const content = () => captureContent("abc");
export function navigationWire(
  state: NavigationState = "prepared",
  pending = false,
) {
  return {
    ...golden,
    state,
    pending,
    locations: state === "retained" || state === "release_unknown" ||
        state === "released"
      ? golden.locations
      : [],
  };
}
export async function preparedNavigation() {
  const f = await opened();
  const captured = await content();
  const preparing = f.owner.prepareNavigation(
    captured,
    golden.position,
    "definition",
  );
  f.reply(2, navigationWire());
  const operation = await preparing;
  const source = f.registry.navigations;
  return { ...f, captured, operation, source, row: source.get().rows[0]! };
}
