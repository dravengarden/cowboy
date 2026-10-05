import type { EditorPanelItem, EditorPort } from "../editorExtensions/contract";
import {
  createEditorPluginInstance,
  type EditorPluginCommandSpec,
  type EditorPluginInstance,
  type EditorPluginInstanceOptions,
  type EditorPluginPanelSpec,
} from "./instance";
import {
  compareEditorPluginVersions,
  EDITOR_PLUGIN_LIMITS,
  editorPluginIncompatibility,
  type EditorPluginManifest,
  type EditorPluginPackage,
  type EditorPluginPermission,
  type EditorPluginSettingValues,
  normalizeEditorPluginSettings,
  parseEditorPluginManifest,
  parseEditorPluginPackage,
} from "./manifest";
import type { EditorPluginTransport } from "./sandbox";

/** Installed state for one principal on this device. */
interface StoredPlugin {
  readonly pkg: EditorPluginPackage;
  /** The version replaced by the last upgrade, kept for automatic rollback. */
  readonly previous: EditorPluginPackage | null;
  readonly enabled: boolean;
  readonly settings: EditorPluginSettingValues;
  readonly data: unknown;
  readonly failure: string | null;
  readonly installedAt: number;
  readonly updatedAt: number;
}
export interface EditorPluginHistoryEntry {
  readonly at: number;
  readonly plugin: string;
  readonly version: string;
  readonly action:
    | "install"
    | "upgrade"
    | "downgrade"
    | "reinstall"
    | "enable"
    | "disable"
    | "uninstall"
    | "rollback"
    | "failure";
  readonly detail?: string;
}
interface StoredState {
  readonly schema: 1;
  readonly plugins: readonly StoredPlugin[];
  readonly history: readonly EditorPluginHistoryEntry[];
}

export interface EditorPluginPersistence {
  load(): Promise<unknown>;
  save(value: StoredState): Promise<void>;
}

export type EditorPluginStatus = "running" | "starting" | "disabled" | "failed";
export interface EditorPluginView {
  readonly manifest: EditorPluginManifest;
  readonly digest: string;
  readonly enabled: boolean;
  readonly status: EditorPluginStatus;
  readonly failure: string | null;
  readonly settings: EditorPluginSettingValues;
  readonly previousVersion: string | null;
  readonly commands: readonly EditorPluginCommandSpec[];
  readonly panels: readonly EditorPluginPanelSpec[];
}
export interface EditorPluginSnapshot {
  readonly loaded: boolean;
  readonly plugins: readonly EditorPluginView[];
  readonly history: readonly EditorPluginHistoryEntry[];
}

export interface EditorPluginInstallPlan {
  readonly pkg: EditorPluginPackage;
  readonly kind: "install" | "upgrade" | "downgrade" | "reinstall";
  readonly installedVersion: string | null;
  readonly incompatibility: string | null;
  /** Permissions this package adds beyond the installed version's. */
  readonly newPermissions: readonly EditorPluginPermission[];
}

const EMPTY: StoredState = { schema: 1, plugins: [], history: [] };

function decodeState(value: unknown): StoredState {
  if (!value || typeof value !== "object") return EMPTY;
  const v = value as Record<string, unknown>;
  if (v.schema !== 1 || !Array.isArray(v.plugins)) return EMPTY;
  const plugins: StoredPlugin[] = [];
  for (const raw of v.plugins.slice(0, EDITOR_PLUGIN_LIMITS.installed)) {
    try {
      const p = raw as StoredPlugin;
      // Stored packages were verified at install; the manifest is re-validated
      // so a corrupted record cannot widen its permissions.
      const manifest = parseEditorPluginManifest(p.pkg.manifest);
      if (typeof p.pkg.main !== "string" || typeof p.pkg.digest !== "string") {
        continue;
      }
      const pkg = { ...p.pkg, manifest };
      let previous: EditorPluginPackage | null = null;
      if (p.previous) {
        try {
          previous = {
            ...p.previous,
            manifest: parseEditorPluginManifest(p.previous.manifest),
          };
        } catch { /* A broken rollback copy only removes rollback. */ }
      }
      plugins.push({
        pkg,
        previous,
        enabled: p.enabled === true,
        settings: normalizeEditorPluginSettings(manifest, p.settings),
        data: p.data ?? null,
        failure: typeof p.failure === "string" ? p.failure : null,
        installedAt: Number(p.installedAt) || 0,
        updatedAt: Number(p.updatedAt) || 0,
      });
    } catch { /* Skip one unreadable record; others still load. */ }
  }
  const history = Array.isArray(v.history)
    ? (v.history as EditorPluginHistoryEntry[]).slice(-100)
    : [];
  return { schema: 1, plugins, history };
}

export interface EditorPluginHostOptions {
  readonly persistence: EditorPluginPersistence;
  readonly sandbox: (main: string) => EditorPluginTransport;
  readonly notice: (text: string) => void;
  readonly now?: () => number;
  readonly timeouts?: EditorPluginInstanceOptions["timeouts"];
}

/** Install, enable, upgrade, roll back and uninstall editor plugins, and run
 * the enabled ones. One host serves every Draft and Session editor. */
export function createEditorPluginHost(options: EditorPluginHostOptions) {
  const now = options.now ?? Date.now;
  let state: StoredState = EMPTY;
  let loaded = false;
  let loading: Promise<void> | null = null;
  let queue: Promise<unknown> = Promise.resolve();
  const instances = new Map<string, EditorPluginInstance>();
  const starting = new Set<string>();
  const listeners = new Set<() => void>();
  let snapshot: EditorPluginSnapshot = {
    loaded: false,
    plugins: [],
    history: [],
  };

  const publish = (): void => {
    snapshot = {
      loaded,
      history: state.history,
      plugins: state.plugins.map((p): EditorPluginView => {
        const instance = instances.get(p.pkg.manifest.id);
        const running = !!instance?.running && !starting.has(p.pkg.manifest.id);
        return {
          manifest: p.pkg.manifest,
          digest: p.pkg.digest,
          enabled: p.enabled,
          status: !p.enabled
            ? (p.failure ? "failed" : "disabled")
            : starting.has(p.pkg.manifest.id)
            ? "starting"
            : running
            ? "running"
            : "failed",
          failure: p.failure,
          settings: p.settings,
          previousVersion: p.previous?.manifest.version ?? null,
          commands: running ? instance!.commands() : [],
          panels: running ? instance!.panels() : [],
        };
      }),
    };
    for (const listener of listeners) listener();
  };

  const serial = <T>(task: () => Promise<T>): Promise<T> => {
    const next = queue.then(task, task);
    queue = next.catch(() => undefined);
    return next;
  };

  // Every write derives from the latest committed state, in order. Lifecycle
  // steps and a running plugin's saveData() can therefore interleave without
  // one overwriting the other, and saveData() never waits on a lifecycle step.
  let writes: Promise<unknown> = Promise.resolve();
  const persist = (
    change: (current: StoredState) => StoredState,
  ): Promise<void> => {
    const write = async (): Promise<void> => {
      const next = change(state);
      await options.persistence.save(next);
      state = next;
      publish();
    };
    const result = writes.then(write, write);
    writes = result.catch(() => undefined);
    return result;
  };
  const withPlugin = (
    id: string,
    change: (p: StoredPlugin) => StoredPlugin | null,
    entry?: Omit<EditorPluginHistoryEntry, "at" | "plugin">,
  ) =>
  (current: StoredState): StoredState => ({
    schema: 1,
    plugins: current.plugins.flatMap((p) => {
      if (p.pkg.manifest.id !== id) return [p];
      const changed = change(p);
      return changed ? [changed] : [];
    }),
    history: entry
      ? [...current.history, { at: now(), plugin: id, ...entry }].slice(-100)
      : current.history,
  });
  const find = (id: string): StoredPlugin | undefined =>
    state.plugins.find((p) => p.pkg.manifest.id === id);

  const recordFailure = async (id: string, message: string): Promise<void> => {
    const plugin = find(id);
    if (!plugin) return;
    await persist(withPlugin(
      id,
      (p) => ({ ...p, enabled: false, failure: message, updatedAt: now() }),
      {
        version: plugin.pkg.manifest.version,
        action: "failure",
        detail: message,
      },
    ));
  };

  /** Start one plugin. Never throws: a failure stays with that plugin. */
  const start = async (plugin: StoredPlugin): Promise<string | null> => {
    const id = plugin.pkg.manifest.id;
    await instances.get(id)?.unload();
    instances.delete(id);
    const incompatible = editorPluginIncompatibility(plugin.pkg.manifest);
    if (incompatible) return `Incompatible: ${incompatible}`;
    starting.add(id);
    publish();
    let failed: string | null = null;
    let started = false;
    let transport: EditorPluginTransport;
    try {
      transport = options.sandbox(plugin.pkg.main);
    } catch (error) {
      starting.delete(id);
      return error instanceof Error
        ? error.message
        : "Could not create the plugin sandbox";
    }
    const instance = createEditorPluginInstance({
      pkg: plugin.pkg,
      settings: plugin.settings,
      transport,
      ...(options.timeouts ? { timeouts: options.timeouts } : {}),
      loadData: () => find(id)?.data ?? null,
      saveData: (value) =>
        persist(withPlugin(id, (p) => ({ ...p, data: value }))),
      notice: options.notice,
      fail: (message) => {
        failed ??= message;
        // A start failure is returned to the caller, which owns rollback.
        if (!started || instances.get(id) !== instance) return;
        instances.delete(id);
        options.notice(`${plugin.pkg.manifest.name}: ${message}`);
        void serial(() => recordFailure(id, message));
      },
      changed: publish,
    });
    instances.set(id, instance);
    try {
      await instance.start();
      started = failed === null;
    } catch (error) {
      failed ??= error instanceof Error
        ? error.message
        : "The plugin failed to load";
    }
    starting.delete(id);
    if (failed) {
      if (instances.get(id) === instance) instances.delete(id);
      await instance.unload();
    }
    publish();
    return failed;
  };

  let startup: Promise<void> | null = null;
  /** Read installed state once. Lifecycle steps call this inside `serial`, so
   * it must not wait on the queue itself. */
  const ensureState = (): Promise<void> => {
    loading ??= (async () => {
      try {
        state = decodeState(await options.persistence.load());
      } catch (error) {
        // Never treat unreadable storage as "nothing installed": a later
        // install would overwrite the real record. Retry on the next use.
        loading = null;
        throw error;
      }
      loaded = true;
      publish();
      startup = serial(async () => {
        for (const plugin of state.plugins) {
          if (!plugin.enabled) continue;
          const failure = await start(plugin);
          if (failure) await recordFailure(plugin.pkg.manifest.id, failure);
        }
      });
    })();
    return loading;
  };
  const ensureLoaded = async (): Promise<void> => {
    await ensureState();
    await startup;
  };

  const planFor = (pkg: EditorPluginPackage): EditorPluginInstallPlan => {
    const installed = find(pkg.manifest.id);
    const order = installed
      ? compareEditorPluginVersions(
        pkg.manifest.version,
        installed.pkg.manifest.version,
      )
      : 1;
    return {
      pkg,
      kind: !installed
        ? "install"
        : order > 0
        ? "upgrade"
        : order < 0
        ? "downgrade"
        : "reinstall",
      installedVersion: installed?.pkg.manifest.version ?? null,
      incompatibility: editorPluginIncompatibility(pkg.manifest),
      newPermissions: pkg.manifest.permissions.filter((permission) =>
        !installed?.pkg.manifest.permissions.includes(permission)
      ),
    };
  };

  const host = {
    subscribe(listener: () => void): () => void {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    getSnapshot: (): EditorPluginSnapshot => snapshot,
    ensureLoaded,
    /** Validate a package file and describe what installing it would do. */
    async inspect(source: string): Promise<EditorPluginInstallPlan> {
      await ensureState();
      return planFor(await parseEditorPluginPackage(source));
    },
    /** Install or replace a package the user reviewed through `inspect`.
     * A failing upgrade restores the previous version and its running state. */
    install(
      plan: EditorPluginInstallPlan,
    ): Promise<{ ok: boolean; message: string }> {
      return serial(async () => {
        await ensureState();
        const { pkg } = plan;
        const id = pkg.manifest.id;
        if (plan.incompatibility) throw new Error(plan.incompatibility);
        const current = planFor(pkg);
        if (
          current.installedVersion !== plan.installedVersion ||
          current.newPermissions.length !== plan.newPermissions.length
        ) {
          throw new Error(
            "The installed plugins changed; review the package again.",
          );
        }
        const installed = find(id);
        if (
          !installed && state.plugins.length >= EDITOR_PLUGIN_LIMITS.installed
        ) {
          throw new Error(
            `At most ${EDITOR_PLUGIN_LIMITS.installed} plugins can be installed`,
          );
        }
        const at = now();
        const next: StoredPlugin = {
          pkg,
          previous: installed && installed.pkg.digest !== pkg.digest
            ? installed.pkg
            : installed?.previous ?? null,
          enabled: true,
          settings: normalizeEditorPluginSettings(
            pkg.manifest,
            installed?.settings,
          ),
          data: installed?.data ?? null,
          failure: null,
          installedAt: installed?.installedAt ?? at,
          updatedAt: at,
        };
        await persist((current) => ({
          schema: 1,
          plugins: installed
            ? current.plugins.map((p) => (p.pkg.manifest.id === id ? next : p))
            : [...current.plugins, next],
          history: [
            ...current.history,
            {
              at,
              plugin: id,
              version: pkg.manifest.version,
              action: plan.kind,
            },
          ].slice(-100),
        }));
        const failure = await start(next);
        if (!failure) {
          return {
            ok: true,
            message: `${pkg.manifest.name} ${pkg.manifest.version} is enabled.`,
          };
        }
        if (installed && installed.pkg.digest !== pkg.digest) {
          // Roll back to the exact previous package, settings and enabled state.
          await persist(withPlugin(
            id,
            () => ({ ...installed, failure: null, updatedAt: now() }),
            {
              version: installed.pkg.manifest.version,
              action: "rollback",
              detail: `${pkg.manifest.version} failed: ${failure}`,
            },
          ));
          if (installed.enabled) {
            const again = await start(installed);
            if (again) await recordFailure(id, again);
          }
          return {
            ok: false,
            message:
              `${pkg.manifest.name} ${pkg.manifest.version} failed (${failure}); ${installed.pkg.manifest.version} was restored.`,
          };
        }
        await recordFailure(id, failure);
        return {
          ok: false,
          message: `${pkg.manifest.name} failed to start: ${failure}`,
        };
      });
    },
    setEnabled(id: string, enabled: boolean): Promise<void> {
      return serial(async () => {
        await ensureState();
        const plugin = find(id);
        if (!plugin || plugin.enabled === enabled && !plugin.failure) return;
        if (!enabled) {
          await instances.get(id)?.unload();
          instances.delete(id);
        }
        await persist(withPlugin(
          id,
          (p) => ({ ...p, enabled, failure: null, updatedAt: now() }),
          {
            version: plugin.pkg.manifest.version,
            action: enabled ? "enable" : "disable",
          },
        ));
        if (enabled) {
          const failure = await start(find(id)!);
          if (failure) await recordFailure(id, failure);
        }
      });
    },
    uninstall(id: string): Promise<void> {
      return serial(async () => {
        await ensureState();
        const plugin = find(id);
        if (!plugin) return;
        await instances.get(id)?.unload();
        instances.delete(id);
        // Code, rollback copy, settings and plugin data all go together.
        await persist(withPlugin(id, () => null, {
          version: plugin.pkg.manifest.version,
          action: "uninstall",
        }));
      });
    },
    updateSettings(id: string, values: Record<string, unknown>): Promise<void> {
      return serial(async () => {
        await ensureState();
        const plugin = find(id);
        if (!plugin) return;
        const settings = normalizeEditorPluginSettings(plugin.pkg.manifest, {
          ...plugin.settings,
          ...values,
        });
        await persist(withPlugin(id, (p) => ({ ...p, settings })));
        instances.get(id)?.updateSettings(settings);
      });
    },
    /** Run a command against the editor the user invoked it from. */
    async runCommand(
      pluginId: string,
      commandId: string,
      port: EditorPort,
    ): Promise<void> {
      const instance = instances.get(pluginId);
      if (!instance?.running) throw new Error("This plugin is not running");
      await instance.runCommand(commandId, port);
    },
    async renderPanel(
      pluginId: string,
      panelId: string,
      port: EditorPort,
    ): Promise<readonly EditorPanelItem[]> {
      const instance = instances.get(pluginId);
      if (!instance?.running) return [];
      return await instance.renderPanel(panelId, port);
    },
    appliesTo(pluginId: string, port: EditorPort): boolean {
      return instances.get(pluginId)?.applies(port) ?? false;
    },
    /** Stop every running plugin (sign-out or hot disposal). */
    async dispose(): Promise<void> {
      await Promise.all(
        [...instances.values()].map((instance) => instance.unload()),
      );
      instances.clear();
      publish();
    },
  };
  return host;
}

export type EditorPluginHost = ReturnType<typeof createEditorPluginHost>;
