import assert from "node:assert/strict";
import test from "node:test";
import { configurationEquivalence, installConfigurationEquivalence } from "./configuration.mjs";

const KEY = "cowboy.dev/configuration-equivalence";
test("only exact Fable 5.1 spellings are equivalent; native values stay truthful", () => {
  for (const value of ["claude-fable-5-1", "claude-fable-5-1[1m]"]) {
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
    assert.equal(options[0]._meta[KEY].value, value);
    assert.equal(options[0]._meta[KEY].equivalent_values.length, 1);
    assert.notEqual(options[0]._meta[KEY].equivalent_values[0], value);
    assert.equal(options[1]._meta, undefined);
    for (const other of [
      "opus",
      "claude-opus-4-6",
      "claude-fable-5",
      "claude-fable-5-2",
      "custom-fable-5-1",
    ]) {
      options[0].currentValue = other;
      configurationEquivalence(options);
      assert.equal(options[0]._meta[KEY], undefined);
    }
  }
  configurationEquivalence(undefined);
});

test("creation, resume, configuration replies and notifications carry native equivalence", async () => {
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
        configOptions: [{ id: "model", currentValue: "claude-fable-5-1" }],
      };
    }
    async resumeSession() {
      return this.createSession();
    }
    async setSessionConfigOption() {
      return this.createSession();
    }
  }
  installConfigurationEquivalence(Agent);
  const agent = new Agent();
  assert.equal(await agent.initialize(), "initialized");
  const sender = agent.client.sessionUpdate;
  await agent.initialize();
  assert.equal(agent.client.sessionUpdate, sender);
  for (const method of ["createSession", "resumeSession", "setSessionConfigOption"]) {
    const result = await agent[method]();
    assert.equal(result.configOptions[0]._meta[KEY].value, "claude-fable-5-1");
  }
  const options = [{ id: "model", currentValue: "claude-fable-5-1[1m]" }];
  assert.equal(
    await agent.client.sessionUpdate({
      update: { sessionUpdate: "config_option_update", configOptions: options },
    }),
    "sent",
  );
  assert.equal(sent.update.configOptions[0]._meta[KEY].value, "claude-fable-5-1[1m]");
});
