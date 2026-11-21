// Native Fable 5.1 and 5.2 each have a 1M window with or without the [1m]
// spelling. CLI 2.1.326 lists Fable 5.2 as `claude-fable-5-2[1m]` on a fresh
// session and may emit `claude-fable-5-2` on resume, which ACP 0.107.3's
// `aliasResumedModelRow` rewrites to `fable`. Keep the actual value; advertise
// only each exact, provider-owned equivalence class (never across versions).
// The family alias names whatever model the CLI resolves it to, so it joins a
// class only while this session's own `fable` row is the version the class
// pins: a newer CLI that moves the alias stops confirming instead of drifting.
// https://code.claude.com/docs/en/model-config#extended-context
const EQUIVALENCES = [
  { values: ["claude-fable-5-1", "claude-fable-5-1[1m]"] },
  {
    values: ["claude-fable-5-2", "claude-fable-5-2[1m]"],
    alias: { value: "fable", name: /^Fable 5\.2(?![.\d])/ },
  },
];
const KEY = "cowboy.dev/configuration-equivalence";
const OPTIONAL_SESSION_METHODS = [
  "withInitialConfigOptions",
  "newSession",
  "loadSession",
  "resumeSession",
  "unstable_forkSession",
  "sessionOptions",
];

function advertised(option, value) {
  const entries = (Array.isArray(option.options) ? option.options : []).flatMap((entry) =>
    Array.isArray(entry?.options) ? entry.options : [entry],
  );
  return entries.find((entry) => entry?.value === value);
}

function members(group, option) {
  const alias = group.alias && advertised(option, group.alias.value);
  return typeof alias?.name === "string" && group.alias.name.test(alias.name)
    ? [...group.values, group.alias.value]
    : group.values;
}

export function configurationEquivalence(options) {
  if (!Array.isArray(options)) return;
  for (const option of options) {
    if (option.id !== "model") continue;
    // Rebuilding a native option may retain metadata from its previous value.
    if (option._meta) delete option._meta[KEY];
    const group = EQUIVALENCES.map((candidate) => members(candidate, option)).find((values) =>
      values.includes(option.currentValue),
    );
    if (!group) continue;
    option._meta = {
      ...option._meta,
      [KEY]: {
        schema: 1,
        value: option.currentValue,
        equivalent_values: group.filter((value) => value !== option.currentValue),
      },
    };
  }
}

export function installConfigurationEquivalence(Agent) {
  for (const method of ["createSession", "setSessionConfigOption"]) {
    const original = Agent.prototype[method];
    Agent.prototype[method] = async function (...args) {
      const result = await original.apply(this, args);
      configurationEquivalence(result?.configOptions);
      return result;
    };
  }
  for (const method of OPTIONAL_SESSION_METHODS) {
    const original = Agent.prototype[method];
    if (typeof original !== "function") continue;
    Agent.prototype[method] = async function (...args) {
      const result = await original.apply(this, args);
      configurationEquivalence(result?.configOptions);
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
