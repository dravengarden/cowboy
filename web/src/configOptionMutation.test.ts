import { assertEquals } from "jsr:@std/assert";
import type { ConfigOption } from "./protocol.ts";
import { configOptionsMatchChanges } from "./configOptionMutation.ts";

const options: ConfigOption[] = [{
  id: "model",
  name: "Model",
  currentValue: "gpt-5.6-sol",
  options: [],
}, {
  id: "full_access",
  name: "Full access",
  currentValue: true,
  options: [],
}];

Deno.test("config acknowledgement requires every requested value", () => {
  assertEquals(
    configOptionsMatchChanges(options, [
      { configId: "model", value: "gpt-5.6-sol" },
      { configId: "full_access", value: true },
    ]),
    true,
  );
  assertEquals(
    configOptionsMatchChanges(options, [
      { configId: "model", value: "gpt-6-astra" },
    ]),
    false,
  );
  assertEquals(
    configOptionsMatchChanges(options, [
      { configId: "missing", value: "value" },
    ]),
    false,
  );
});
