import { assertEquals } from "jsr:@std/assert";
import { convergeArguments, parseInvocation } from "./converge-machine.ts";

Deno.test("a dry run is the default and --apply is the only way to submit", () => {
  assertEquals(
    convergeArguments({ machines: ["hawk"], plugins: [], apply: false }),
    ["operator", "converge", "--machine", "hawk"],
  );
  assertEquals(
    convergeArguments({ machines: ["hawk"], plugins: [], apply: true }),
    ["operator", "converge", "--machine", "hawk", "--apply"],
  );
});

Deno.test("the requested Machine order survives into the rollout", () => {
  // The first Machine is the canary; reordering it would change which host
  // absorbs a bad release first.
  assertEquals(
    convergeArguments({
      machines: ["hawk", "falcon", "macbook-air"],
      plugins: ["codex"],
      apply: true,
    }),
    [
      "operator",
      "converge",
      "--machine",
      "hawk",
      "--machine",
      "falcon",
      "--machine",
      "macbook-air",
      "--plugin",
      "codex",
      "--apply",
    ],
  );
});

Deno.test("Machines are accepted positionally or by flag", () => {
  const parsed = parseInvocation(["hawk", "--machine", "falcon", "--apply"]);
  assertEquals("error" in parsed, false);
  if ("error" in parsed) return;
  assertEquals(parsed.invocation.machines, ["hawk", "falcon"]);
  assertEquals(parsed.invocation.apply, true);
  assertEquals(parsed.cowboy, "cowboy");
});

Deno.test("no Machine at all converges every connected Machine", () => {
  const parsed = parseInvocation([]);
  assertEquals("error" in parsed, false);
  if ("error" in parsed) return;
  assertEquals(parsed.invocation.machines, []);
  assertEquals(convergeArguments(parsed.invocation), ["operator", "converge"]);
});

Deno.test("a typo is refused rather than widened into a bigger run", () => {
  assertEquals(parseInvocation(["--aply"]), { error: "unknown option --aply" });
  assertEquals(parseInvocation(["--plugin"]), {
    error: "--plugin needs a value",
  });
  assertEquals(parseInvocation(["--machine", "--apply"]), {
    error: "--machine needs a value",
  });
});

Deno.test("a custom cowboy path is used to run, not passed to it", () => {
  const parsed = parseInvocation([
    "hawk",
    "--cowboy",
    "/opt/cowboy/bin/cowboy",
  ]);
  assertEquals("error" in parsed, false);
  if ("error" in parsed) return;
  assertEquals(parsed.cowboy, "/opt/cowboy/bin/cowboy");
  assertEquals(convergeArguments(parsed.invocation), [
    "operator",
    "converge",
    "--machine",
    "hawk",
  ]);
});
