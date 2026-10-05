/** The plugin-side runtime, evaluated in the sandbox Worker before the
 * plugin's own `main`. It is plain JavaScript text because it runs in a
 * separate, opaque-origin realm; it shares no module or object with the App.
 *
 * Authoring API (see examples/editor-plugins/text-tools/main.js):
 *
 *   definePlugin({
 *     onload(ctx) {
 *       ctx.addCommand({ id, title, description?, icon?, toolbar?, run(editor) });
 *       ctx.addPanel({ id, title, render(doc) });
 *       ctx.settings; ctx.onSettingsChange(fn);
 *       await ctx.loadData(); await ctx.saveData(value); ctx.notice(text);
 *     },
 *     onunload() {},
 *   });
 *
 * `editor.read()` returns `{ text, selection, revision }` when the plugin holds
 * `editor:read`; `await editor.replaceSelection(text)` needs `editor:write` and
 * resolves false when the user kept typing, an IME owns the editor, the
 * command finished or the document changed. */
export const EDITOR_PLUGIN_WORKER_PRELUDE = String.raw`"use strict";
(() => {
  for (const name of ["fetch", "XMLHttpRequest", "WebSocket", "EventSource", "importScripts", "indexedDB", "caches", "BroadcastChannel"]) {
    try { Object.defineProperty(self, name, { value: undefined, configurable: false }); } catch {}
  }
  let port = null;
  let definition = null;
  let settings = {};
  let nextCall = 1;
  const pending = new Map();
  const commands = new Map();
  const panels = new Map();
  const settingsListeners = new Set();
  const send = (message) => port && port.postMessage(message);
  const request = (op, args) => new Promise((resolve, reject) => {
    const call = nextCall++;
    pending.set(call, { resolve, reject });
    send({ t: "request", call, op, args });
  });
  const errorText = (error) => error instanceof Error ? error.message : String(error);
  const reply = (call, run) => {
    Promise.resolve().then(run).then(
      (value) => send({ t: "done", call, ok: true, value: value === undefined ? null : value }),
      (error) => send({ t: "done", call, ok: false, error: errorText(error).slice(0, 500) }),
    );
  };
  const freeze = (value) => JSON.parse(JSON.stringify(value));
  const editorFor = (invocation) => ({
    context: invocation.context,
    read() {
      if (!invocation.snapshot) throw new Error("This plugin does not have editor:read permission");
      return invocation.snapshot;
    },
    async replaceSelection(text) {
      if (typeof text !== "string") throw new TypeError("replaceSelection needs text");
      const result = await request("replaceSelection", { token: invocation.token, text });
      if (result && result.snapshot) invocation.snapshot = result.snapshot;
      return !!(result && result.applied);
    },
  });
  const ctx = {
    get settings() { return settings; },
    onSettingsChange(listener) { settingsListeners.add(listener); return () => settingsListeners.delete(listener); },
    addCommand(spec) {
      if (!spec || typeof spec.run !== "function") throw new TypeError("addCommand needs run()");
      const declared = { id: spec.id, title: spec.title, description: spec.description, icon: spec.icon, toolbar: !!spec.toolbar };
      commands.set(spec.id, spec.run);
      send({ t: "register", kind: "command", spec: declared });
    },
    addPanel(spec) {
      if (!spec || typeof spec.render !== "function") throw new TypeError("addPanel needs render()");
      panels.set(spec.id, spec.render);
      send({ t: "register", kind: "panel", spec: { id: spec.id, title: spec.title } });
    },
    loadData() { return request("loadData", null); },
    saveData(value) { return request("saveData", { value: freeze(value === undefined ? null : value) }); },
    notice(text) { send({ t: "notice", text: String(text).slice(0, 300) }); },
  };
  self.definePlugin = (value) => {
    if (definition) throw new Error("definePlugin may be called once");
    if (!value || typeof value.onload !== "function") throw new TypeError("definePlugin needs onload()");
    definition = value;
  };
  self.addEventListener("error", (event) => {
    send({ t: "crash", error: String(event.message || "Uncaught error").slice(0, 500) });
  });
  self.addEventListener("unhandledrejection", (event) => {
    send({ t: "crash", error: errorText(event.reason).slice(0, 500) });
  });
  const handle = (message) => {
    switch (message.t) {
      case "load":
        settings = Object.freeze({ ...message.settings });
        reply(message.call, async () => {
          if (!definition) throw new Error("The package never called definePlugin()");
          await definition.onload(ctx);
        });
        break;
      case "settings":
        settings = Object.freeze({ ...message.settings });
        for (const listener of settingsListeners) {
          try { listener(settings); } catch (error) { send({ t: "crash", error: errorText(error) }); }
        }
        break;
      case "invoke": {
        const run = commands.get(message.command);
        reply(message.call, () => {
          if (!run) throw new Error("Unknown command");
          return Promise.resolve(run(editorFor(message.invocation))).then(() => null);
        });
        break;
      }
      case "panel": {
        const render = panels.get(message.panel);
        reply(message.call, () => {
          if (!render) throw new Error("Unknown panel");
          return Promise.resolve(render(message.snapshot ?? { context: message.context })).then(freeze);
        });
        break;
      }
      case "unload":
        reply(message.call, () => definition && definition.onunload ? definition.onunload() : undefined);
        break;
      case "reply": {
        const entry = pending.get(message.call);
        if (!entry) return;
        pending.delete(message.call);
        if (message.ok) entry.resolve(message.value);
        else entry.reject(new Error(message.error));
        break;
      }
    }
  };
  self.addEventListener("message", (event) => {
    if (port || !event.ports || event.ports.length !== 1) return;
    port = event.ports[0];
    port.onmessage = (e) => handle(e.data);
    send({ t: "ready" });
  });
})();
`;
