import type {
  EditorPanelItem,
  EditorPort,
  EditorSnapshot,
} from "../editorExtensions/contract";
import {
  EDITOR_PLUGIN_ICONS,
  EDITOR_PLUGIN_LIMITS,
  type EditorPluginIcon,
  type EditorPluginPackage,
  type EditorPluginSettingValues,
} from "./manifest";
import type { EditorPluginTransport } from "./sandbox";

export interface EditorPluginCommandSpec {
  readonly id: string;
  readonly title: string;
  readonly description?: string;
  readonly icon?: EditorPluginIcon;
  readonly toolbar: boolean;
}
export interface EditorPluginPanelSpec {
  readonly id: string;
  readonly title: string;
}

export const EDITOR_PLUGIN_TIMEOUTS = {
  start: 5000,
  command: 10000,
  panel: 2000,
  unload: 1000,
} as const;
const MAX_CONTRIBUTIONS = 64;
const MAX_REPLACEMENT = 1024 * 1024;
const MAX_CRASHES = 3;

export interface EditorPluginInstanceOptions {
  readonly pkg: EditorPluginPackage;
  readonly settings: EditorPluginSettingValues;
  readonly transport: EditorPluginTransport;
  readonly timeouts?: {
    readonly [K in keyof typeof EDITOR_PLUGIN_TIMEOUTS]: number;
  };
  loadData(): unknown;
  saveData(value: unknown): Promise<void>;
  notice(text: string): void;
  /** The plugin can no longer be trusted to run: it crashed repeatedly or
   * stopped answering. The owner records the failure and stops it. */
  fail(message: string): void;
  changed(): void;
}

interface Invocation {
  readonly port: EditorPort;
  snapshot: EditorSnapshot;
  live: boolean;
}

class PluginCallError extends Error {}

/** One running plugin: a closed message protocol over its sandbox. Every edit
 * goes through the host's version-bound EditorPort with an invocation token
 * that dies when the command settles, times out or the plugin stops. */
export function createEditorPluginInstance(
  options: EditorPluginInstanceOptions,
) {
  const { pkg, transport } = options;
  const timeouts = options.timeouts ?? EDITOR_PLUGIN_TIMEOUTS;
  const permissions = new Set(pkg.manifest.permissions);
  const commands = new Map<string, EditorPluginCommandSpec>();
  const panels = new Map<string, EditorPluginPanelSpec>();
  const calls = new Map<
    number,
    { resolve: (value: unknown) => void; reject: (error: Error) => void }
  >();
  const invocations = new Map<string, Invocation>();
  let nextCall = 1;
  let ready = false;
  let loaded = false;
  let stopped = false;
  let crashes = 0;
  const readyWaiters: (() => void)[] = [];

  const stop = (): void => {
    if (stopped) return;
    stopped = true;
    for (const invocation of invocations.values()) invocation.live = false;
    invocations.clear();
    for (const call of calls.values()) {
      call.reject(new PluginCallError("The plugin stopped"));
    }
    calls.clear();
    transport.terminate();
  };
  const failHard = (message: string): void => {
    if (stopped) return;
    stop();
    options.fail(message);
  };

  const call = (
    message: Record<string, unknown>,
    timeout: number,
  ): Promise<unknown> => {
    if (stopped) {
      return Promise.reject(new PluginCallError("The plugin is not running"));
    }
    const id = nextCall++;
    return new Promise((resolve, reject) => {
      const timer = globalThis.setTimeout(() => {
        calls.delete(id);
        reject(new PluginCallError("The plugin did not answer in time"));
        // A plugin that cannot answer may be stuck in a loop; its Worker is
        // terminated so it cannot hold CPU or a stale invocation.
        failHard("The plugin stopped responding and was turned off.");
      }, timeout);
      calls.set(id, {
        resolve: (value) => {
          globalThis.clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          globalThis.clearTimeout(timer);
          reject(error);
        },
      });
      transport.post({ ...message, call: id });
    });
  };

  const validId = (id: unknown): id is string =>
    typeof id === "string" && /^[a-z][a-z0-9-]{0,47}$/.test(id);
  const label = (value: unknown, max: number): string | null =>
    typeof value === "string" && value.trim() !== "" && value.length <= max
      ? value
      : null;

  const register = (kind: unknown, spec: unknown): void => {
    if (!spec || typeof spec !== "object" || loaded) {
      throw new Error("Contributions must be registered during onload");
    }
    const s = spec as Record<string, unknown>;
    const title = label(s.title, 80);
    if (!validId(s.id) || title === null) {
      throw new Error("Contribution needs an id and title");
    }
    if (commands.size + panels.size >= MAX_CONTRIBUTIONS) {
      throw new Error("Too many contributions");
    }
    if (kind === "command") {
      if (commands.has(s.id)) throw new Error(`Duplicate command "${s.id}"`);
      const description = s.description === undefined
        ? undefined
        : label(s.description, 200);
      if (description === null) throw new Error("Invalid command description");
      if (
        s.icon !== undefined &&
        !EDITOR_PLUGIN_ICONS.includes(s.icon as EditorPluginIcon)
      ) {
        throw new Error(`Unsupported icon "${String(s.icon)}"`);
      }
      commands.set(s.id, {
        id: s.id,
        title,
        ...(description ? { description } : {}),
        ...(s.icon ? { icon: s.icon as EditorPluginIcon } : {}),
        toolbar: s.toolbar === true,
      });
    } else if (kind === "panel") {
      if (panels.has(s.id)) throw new Error(`Duplicate panel "${s.id}"`);
      panels.set(s.id, { id: s.id, title });
    } else {
      throw new Error("Unknown contribution");
    }
  };

  const answer = (id: unknown, run: () => unknown | Promise<unknown>): void => {
    void Promise.resolve().then(run).then(
      (value) =>
        transport.post({
          t: "reply",
          call: id,
          ok: true,
          value: value ?? null,
        }),
      (error: unknown) =>
        transport.post({
          t: "reply",
          call: id,
          ok: false,
          error: error instanceof Error ? error.message : "Request failed",
        }),
    );
  };

  const request = (op: unknown, args: unknown): unknown | Promise<unknown> => {
    const a = (args ?? {}) as Record<string, unknown>;
    switch (op) {
      case "replaceSelection": {
        if (!permissions.has("editor:write")) {
          throw new Error("This plugin does not have editor:write permission");
        }
        const invocation = typeof a.token === "string"
          ? invocations.get(a.token)
          : undefined;
        if (!invocation?.live) return { applied: false };
        if (typeof a.text !== "string" || a.text.length > MAX_REPLACEMENT) {
          throw new Error("Replacement text is too large");
        }
        const applied = invocation.port.replaceSelection(
          a.text,
          invocation.snapshot,
        );
        if (applied) invocation.snapshot = invocation.port.read();
        return {
          applied,
          ...(applied && permissions.has("editor:read")
            ? { snapshot: invocation.snapshot }
            : {}),
        };
      }
      case "loadData":
        return options.loadData() ?? null;
      case "saveData": {
        const value = a.value ?? null;
        if (JSON.stringify(value).length > EDITOR_PLUGIN_LIMITS.dataBytes) {
          throw new Error("Plugin data is larger than 64 KiB");
        }
        return options.saveData(value).then(() => null);
      }
      default:
        throw new Error("Unknown request");
    }
  };

  transport.listen((raw) => {
    if (stopped || !raw || typeof raw !== "object") return;
    const message = raw as Record<string, unknown>;
    switch (message.t) {
      case "ready":
        ready = true;
        for (const waiter of readyWaiters.splice(0)) waiter();
        break;
      case "register":
        try {
          register(message.kind, message.spec);
        } catch (error) {
          failHard(
            error instanceof Error ? error.message : "Invalid contribution",
          );
        }
        break;
      case "done": {
        const entry = calls.get(message.call as number);
        if (!entry) return;
        calls.delete(message.call as number);
        if (message.ok === true) entry.resolve(message.value);
        else {entry.reject(
            new PluginCallError(String(message.error ?? "Plugin error")),
          );}
        break;
      }
      case "request":
        answer(message.call, () => request(message.op, message.args));
        break;
      case "notice":
        if (typeof message.text === "string" && message.text.trim()) {
          options.notice(`${pkg.manifest.name}: ${message.text.slice(0, 300)}`);
        }
        break;
      case "crash":
        if (!loaded || ++crashes >= MAX_CRASHES) {
          failHard(String(message.error ?? "The plugin crashed"));
        } else {
          options.notice(
            `${pkg.manifest.name}: ${String(message.error ?? "error")}`,
          );
        }
        break;
    }
  });

  const invocationFor = (port: EditorPort): [string, Invocation] => {
    const token = crypto.randomUUID();
    const invocation: Invocation = { port, snapshot: port.read(), live: true };
    invocations.set(token, invocation);
    return [token, invocation];
  };
  const visibleSnapshot = (snapshot: EditorSnapshot): EditorSnapshot | null =>
    permissions.has("editor:read") ? snapshot : null;
  const applies = (port: EditorPort): boolean =>
    pkg.manifest.contexts.includes(port.context.kind) &&
    pkg.manifest.surfaces.includes(port.context.surface);

  return {
    pkg,
    async start(): Promise<void> {
      if (!ready) {
        await new Promise<void>((resolve, reject) => {
          const timer = globalThis.setTimeout(() => {
            reject(new Error("The plugin sandbox did not start"));
          }, timeouts.start);
          readyWaiters.push(() => {
            globalThis.clearTimeout(timer);
            resolve();
          });
        });
      }
      await call({ t: "load", settings: options.settings }, timeouts.start);
      if (stopped) throw new Error("The plugin stopped while loading");
      loaded = true;
      options.changed();
    },
    commands:
      (): readonly EditorPluginCommandSpec[] => (loaded
        ? [...commands.values()]
        : []),
    panels:
      (): readonly EditorPluginPanelSpec[] => (loaded
        ? [...panels.values()]
        : []),
    applies,
    async runCommand(id: string, port: EditorPort): Promise<void> {
      if (!loaded || !commands.has(id)) {
        throw new Error("This command is no longer available");
      }
      if (!applies(port)) {
        throw new Error("This plugin does not run in this editor");
      }
      const [token, invocation] = invocationFor(port);
      try {
        await call({
          t: "invoke",
          command: id,
          invocation: {
            token,
            context: port.context,
            snapshot: visibleSnapshot(invocation.snapshot),
          },
        }, timeouts.command);
      } finally {
        invocation.live = false;
        invocations.delete(token);
      }
    },
    async renderPanel(
      id: string,
      port: EditorPort,
    ): Promise<readonly EditorPanelItem[]> {
      if (!loaded || !panels.has(id) || !applies(port)) return [];
      const snapshot = port.read();
      const value = await call({
        t: "panel",
        panel: id,
        context: port.context,
        snapshot: visibleSnapshot(snapshot),
      }, timeouts.panel);
      if (!Array.isArray(value)) throw new Error("A panel must return a list");
      return value.slice(0, 200).flatMap((item, index): EditorPanelItem[] => {
        if (!item || typeof item !== "object") return [];
        const i = item as Record<string, unknown>;
        const text = label(i.label, 200);
        if (text === null) return [];
        const offset =
          typeof i.offset === "number" && Number.isInteger(i.offset) &&
            i.offset >= 0 && i.offset <= snapshot.text.length
            ? i.offset
            : undefined;
        const detail = label(i.detail, 200);
        const depth = typeof i.depth === "number" && Number.isInteger(i.depth)
          ? Math.max(0, Math.min(6, i.depth))
          : undefined;
        return [{
          id: `${id}:${index}`,
          label: text,
          ...(detail ? { detail } : {}),
          ...(offset === undefined ? {} : { offset }),
          ...(depth === undefined ? {} : { depth }),
        }];
      });
    },
    updateSettings(settings: EditorPluginSettingValues): void {
      if (!stopped) transport.post({ t: "settings", settings });
    },
    async unload(): Promise<void> {
      if (stopped) return;
      if (loaded) {
        try {
          await Promise.race([
            call({ t: "unload" }, timeouts.unload + 500),
            new Promise((resolve) =>
              globalThis.setTimeout(resolve, timeouts.unload)
            ),
          ]);
        } catch { /* Unload always terminates the sandbox. */ }
      }
      stop();
    },
    get running(): boolean {
      return !stopped;
    },
  };
}

export type EditorPluginInstance = ReturnType<
  typeof createEditorPluginInstance
>;
