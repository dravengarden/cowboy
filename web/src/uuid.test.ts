import { assertMatch } from "jsr:@std/assert";
import { newUuid } from "./uuid";

Deno.test("newUuid returns a UUID-shaped identifier in the current runtime", () => {
  assertMatch(newUuid(), /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
});
