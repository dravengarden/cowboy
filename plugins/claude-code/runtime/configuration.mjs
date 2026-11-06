// Native Fable 5.1 has a 1M window with or without the historical [1m]
// spelling. The CLI's picker and resumed transcript can use different forms.
// Keep the actual value; advertise only this exact, provider-owned equivalence.
// https://code.claude.com/docs/en/model-config#extended-context
const FABLE = ["claude-fable-5-1", "claude-fable-5-1[1m]"];
const KEY = "cowboy.dev/configuration-equivalence";

export function configurationEquivalence(options) {
  if (!Array.isArray(options)) return;
  for (const option of options) {
    if (option.id !== "model") continue;
    // Rebuilding a native option may retain metadata from its previous value.
    if (option._meta) delete option._meta[KEY];
    if (!FABLE.includes(option.currentValue)) continue;
    option._meta = {
      ...option._meta,
      [KEY]: {
        schema: 1,
        value: option.currentValue,
        equivalent_values: FABLE.filter((value) =>
          value !== option.currentValue
        ),
      },
    };
  }
}

export function installConfigurationEquivalence(Agent) {
  for (const method of ["createSession", "setSessionConfigOption"]) {
    const original = Agent.prototype[method];
    Agent.prototype[method] = async function (...args) {
      const result = await original.apply(this, args);
      configurationEquivalence(result.configOptions);
      return result;
    };
  }
  const initialize = Agent.prototype.initialize;
  Agent.prototype.initialize = async function (...args) {
    if (!this.cowboyConfigurationEquivalence) {
      const update = this.client.sessionUpdate.bind(this.client);
      this.client.sessionUpdate = (notification) => {
        if (notification.update?.sessionUpdate === "config_option_update") {
          configurationEquivalence(notification.update.configOptions);
        }
        return update(notification);
      };
      this.cowboyConfigurationEquivalence = true;
    }
    return initialize.apply(this, args);
  };
}
