import assert from "node:assert/strict";
import test from "node:test";
import { configurationEquivalence, installConfigurationEquivalence } from "./configuration.mjs";

const KEY = "cowboy.dev/configuration-equivalence";
test("only exact per-version Fable spellings are equivalent; native values stay truthful", () => {
  const groups = [
    ["claude-fable-5-1", "claude-fable-5-1[1m]"],
    ["claude-fable-5-2", "claude-fable-5-2[1m]", "fable"],
  ];
  for (const group of groups) {
    const otherGroupValues = groups.filter((candidate) => candidate !== group).flat();
    for (const value of group) {
      const options = [
        {
          id: "model",
          currentValue: value,
          _meta: { native: true },
        },
        { id: "effort", currentValue: "max" },
      ];
      configurationEquivalence(options);
      assert.equal(options[0].currentValue, value);
      assert.equal(options[0]._meta.native, true);
      assert.equal(options[0]._meta[KEY].schema, 1);
      assert.equal(options[0]._meta[KEY].value, value);
      assert.deepEqual(
        options[0]._meta[KEY].equivalent_values,
        group.filter((candidate) => candidate !== value),
      );
      for (const cross of otherGroupValues) {
        assert.equal(options[0]._meta[KEY].equivalent_values.includes(cross), false);
      }
      assert.equal(options[1]._meta, undefined);
      for (const other of [
        "default",
        "opus",
        "claude-opus-4-6",
        "claude-fable-5",
        "claude-fable-5-3",
        "fable[1m]",
        "custom-fable-5-1",
        "custom-fable-5-2",
      ]) {
        options[0].currentValue = other;
        configurationEquivalence(options);
        assert.equal(options[0]._meta[KEY], undefined);
      }
    }
  }
  configurationEquivalence(undefined);
});

test("creation, resume, initial config options, replies and notifications carry native equivalence", async () => {
  let sent;
  class Agent {
    client = {
      sessionUpdate: async (value) => {
        sent = value;
        return "sent";
      },
    };
    async initialize() {
      return "initialized";
    }
    async createSession() {
      return {
        configOptions: [{ id: "model", currentValue: "claude-fable-5-2[1m]" }],
      };
    }
    async withInitialConfigOptions() {
      return {
        configOptions: [{ id: "model", currentValue: "fable" }],
      };
    }
    async resumeSession() {
      return {
        configOptions: [{ id: "model", currentValue: "claude-fable-5-2" }],
      };
    }
    async setSessionConfigOption() {
      return {
        configOptions: [{ id: "model", currentValue: "claude-fable-5-1" }],
      };
    }
  }
  installConfigurationEquivalence(Agent);
  const agent = new Agent();
  assert.equal(await agent.initialize(), "initialized");
  const sender = agent.client.sessionUpdate;
  await agent.initialize();
  assert.equal(agent.client.sessionUpdate, sender);

  const created = await agent.createSession();
  assert.equal(created.configOptions[0]._meta[KEY].value, "claude-fable-5-2[1m]");
  assert.deepEqual(created.configOptions[0]._meta[KEY].equivalent_values, [
    "claude-fable-5-2",
    "fable",
  ]);

  const initial = await agent.withInitialConfigOptions();
  assert.equal(initial.configOptions[0]._meta[KEY].value, "fable");
  assert.deepEqual(initial.configOptions[0]._meta[KEY].equivalent_values, [
    "claude-fable-5-2",
    "claude-fable-5-2[1m]",
  ]);

  const resumed = await agent.resumeSession();
  assert.equal(resumed.configOptions[0]._meta[KEY].value, "claude-fable-5-2");
  assert.deepEqual(resumed.configOptions[0]._meta[KEY].equivalent_values, [
    "claude-fable-5-2[1m]",
    "fable",
  ]);

  const updated = await agent.setSessionConfigOption();
  assert.equal(updated.configOptions[0]._meta[KEY].value, "claude-fable-5-1");
  assert.deepEqual(updated.configOptions[0]._meta[KEY].equivalent_values, ["claude-fable-5-1[1m]"]);

  const options = [{ id: "model", currentValue: "claude-fable-5-2[1m]" }];
  assert.equal(
    await agent.client.sessionUpdate({
      update: { sessionUpdate: "config_option_update", configOptions: options },
    }),
    "sent",
  );
  assert.equal(sent.update.configOptions[0]._meta[KEY].value, "claude-fable-5-2[1m]");
  assert.deepEqual(sent.update.configOptions[0]._meta[KEY].equivalent_values, [
    "claude-fable-5-2",
    "fable",
  ]);
});
