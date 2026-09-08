import { assertEquals } from "jsr:@std/assert";
import type { ClientSpan } from "./otel.ts";
import { ClientOperations } from "./otelOperations.ts";

Deno.test("operation retries keep context and only same-session live echoes count", () => {
  let now = 1;
  const ended: string[] = [];
  const durations: string[] = [];
  let starts = 0;
  const operations = new ClientOperations(
    (name) => {
      starts++;
      return {
        traceparent: `parent-${starts}`,
        end(outcome = "ok") {
          ended.push(`${name}:${outcome}`);
        },
      } as ClientSpan;
    },
    (name, ms) => durations.push(`${name}:${ms}`),
    () => now,
  );
  assertEquals(operations.submit("a", "one"), "parent-1");
  assertEquals(operations.submit("a", "one"), "parent-1");
  assertEquals(operations.submit("b", "one"), undefined);
  operations.acknowledge("b", ["one"]);
  operations.firstOutput("a");
  assertEquals(durations, []);
  now = 11;
  operations.acknowledge("a", ["one"]);
  operations.acknowledge("a", ["one"]);
  operations.userEcho("a", "one");
  now = 21;
  operations.firstOutput("b");
  operations.firstOutput("a");
  operations.firstOutput("a");
  assertEquals(durations, ["command:10", "first_output:10"]);
  assertEquals(ended, ["command:ok", "first_output:ok"]);
});

Deno.test("another client's echo, disconnection and timeout cancel output attribution", () => {
  let now = 1;
  const durations: string[] = [];
  const operations = new ClientOperations(
    () => undefined,
    (name) => durations.push(name),
    () => now,
  );
  operations.submit("a", "one");
  operations.userEcho("a", "one");
  operations.userEcho("a", "another-client");
  operations.firstOutput("a");
  assertEquals(durations, ["command"]);
  operations.submit("a", "two");
  now += 300_001;
  operations.userEcho("a", "two");
  operations.firstOutput("a");
  operations.submit("a", "three");
  operations.clear();
  operations.userEcho("a", "three");
  assertEquals(durations, ["command"]);
});
