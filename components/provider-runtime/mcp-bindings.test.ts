import { test } from "bun:test";
import { assertEquals, assertRejects } from "../../tools/test-assert.ts";
import { prepareMcpBindings } from "./mcp-bindings.mjs";
import { sessionMcpServers, nativeMcpServers } from "../../plugins/claude-code/runtime/mcp.mjs";

const capabilities = {
  schema: 1,
  descriptor_schema: 1,
  relay_schema: 1,
  grant_mcp: true,
  transport: "stdio_tool_only_v1",
};
const reply = () => ({ exitCode: 0, exited: true, output: JSON.stringify(capabilities) });
const plan = () => ({
  schema: 1,
  mcp: {
    schema: 1,
    command: "/immutable/cowboy",
    bindings: [
      { name: "suger", descriptor: "/private/suger.json" },
      { name: "other", descriptor: "/private/other.json" },
    ],
  },
});

test("one exact target probe serves all bindings without overriding native servers", async () => {
  const calls: string[][] = [];
  const entries = await prepareMcpBindings(plan(), {
    servers: ["cowboy_binding_suger", "cowboy_binding_suger_2"],
    probe: async (argv: string[]) => {
      calls.push(argv);
      return reply();
    },
  });
  assertEquals(calls, [["/immutable/cowboy", "mcp", "--binding-capabilities"]]);
  assertEquals(
    entries.map((entry: { name: string }) => entry.name),
    ["cowboy_binding_suger_3", "cowboy_binding_other"],
  );
  assertEquals(entries[0].argv, ["/immutable/cowboy", "mcp", "--binding", "/private/suger.json"]);
  assertEquals(
    entries.every((entry: { placement: string }) => entry.placement === "target"),
    true,
  );
});

test("absent bindings and managed profiles perform no probes", async () => {
  const probe = async () => {
    throw new Error("unexpected probe");
  };
  assertEquals(await prepareMcpBindings(null, { probe }), []);
  assertEquals(await prepareMcpBindings({ schema: 1 }, { probe }), []);
  assertEquals(await prepareMcpBindings({ mcp: "malformed" }, { managed: true, probe }), []);
});

test("invalid launch plans fail before target execution", async () => {
  const invalid: unknown[] = [
    { ...plan().mcp, command: "cowboy" },
    { ...plan().mcp, command: "/binary\0" },
    { ...plan().mcp, env: { SECRET: "forbidden" } },
    { ...plan().mcp, schema: 2 },
    { ...plan().mcp, bindings: [] },
    { ...plan().mcp, bindings: Array(9).fill(plan().mcp.bindings[0]) },
    { ...plan().mcp, bindings: [plan().mcp.bindings[0], plan().mcp.bindings[0]] },
    { ...plan().mcp, bindings: [{ name: "bad.name", descriptor: "/private/path" }] },
    { ...plan().mcp, bindings: [{ name: "suger", descriptor: "relative/path" }] },
    { ...plan().mcp, bindings: [{ name: "suger", descriptor: "/private/path", args: [] }] },
  ];
  let calls = 0;
  for (const mcp of invalid) {
    await assertRejects(
      () =>
        prepareMcpBindings(
          { mcp },
          {
            probe: async () => {
              calls++;
              return reply();
            },
          },
        ),
      Error,
      "Invalid native MCP",
    );
  }
  assertEquals(calls, 0);
});

test("old, partial, oversized or failed capability reports never inject a server", async () => {
  const reports = [
    { ...reply(), exitCode: 1 },
    { ...reply(), exited: false },
    { ...reply(), output_limit: true },
    { ...reply(), output: "x".repeat(4097) },
    { ...reply(), output: "not JSON" },
    { ...reply(), output: JSON.stringify({ schema: 1 }) },
    ...Object.keys(capabilities).map((key) => ({
      ...reply(),
      output: JSON.stringify({ ...capabilities, [key]: key === "grant_mcp" ? false : 2 }),
    })),
    { ...reply(), output: JSON.stringify({ ...capabilities, extra: true }) },
  ];
  for (const result of reports) {
    await assertRejects(() => prepareMcpBindings(plan(), { probe: async () => result }));
  }
  await assertRejects(() => prepareMcpBindings(plan()), Error, "probe unavailable");
});

test("the probed command and descriptor selection are captured before awaiting", async () => {
  const input = plan();
  const entries = await prepareMcpBindings(input, {
    probe: async () => {
      input.mcp.command = "/replacement";
      input.mcp.bindings[0].descriptor = "/replacement";
      return reply();
    },
  });
  assertEquals(entries[0].argv, ["/immutable/cowboy", "mcp", "--binding", "/private/suger.json"]);
});

test("Claude uses existing target stdio routing and keeps managed sessions empty", async () => {
  const target = [
    {
      name: "cowboy_binding_suger",
      placement: "runtime",
      config: { type: "http", url: "https://example.test" },
    },
  ];
  const entries = await sessionMcpServers(target, {
    extensions: plan(),
    managed: false,
    memory: undefined,
    probe: async () => reply(),
  });
  assertEquals(
    entries.find((entry: { name: string; argv: string[] }) => entry.name === "cowboy")?.argv,
    ["/immutable/cowboy", "mcp"],
  );
  assertEquals(
    entries.map((entry: { name: string }) => entry.name),
    ["cowboy_binding_suger", "cowboy_binding_suger_2", "cowboy_binding_other", "cowboy"],
  );
  const configs = nativeMcpServers(entries, { command: "/relay", args: ["--stdio"] });
  assertEquals(configs.cowboy_binding_suger_2, {
    type: "stdio",
    command: "/relay",
    args: ["--stdio", "cowboy_binding_suger_2"],
  });
  assertEquals(
    await sessionMcpServers(target, {
      extensions: plan(),
      managed: true,
      memory: undefined,
      probe: async () => {
        throw new Error("managed probe");
      },
    }),
    [],
  );
});
