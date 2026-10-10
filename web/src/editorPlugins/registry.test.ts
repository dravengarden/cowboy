import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert, assertEquals, assertRejects } from "@std/assert";
import type { EditorPort } from "../editorExtensions/contract.ts";
import {
  EDITOR_PLUGIN_API,
  EDITOR_PLUGIN_PACKAGE_FORMAT,
  editorPluginDigest,
  type EditorPluginManifest,
  parseEditorPluginManifest,
  parseEditorPluginPackage,
} from "./manifest.ts";
import { createEditorPluginHost } from "./registry.ts";
import type { EditorPluginTransport } from "./sandbox.ts";
import { EDITOR_PLUGIN_WORKER_PRELUDE } from "./workerPrelude.ts";

const exampleDir = new URL(
  "../../../examples/editor-plugins/text-tools/",
  import.meta.url,
);

function manifest(
  overrides: Partial<EditorPluginManifest> = {},
): EditorPluginManifest {
  return {
    id: "sample",
    name: "Sample",
    version: "1.0.0",
    description: "Test plugin",
    author: "Tests",
    api: { ...EDITOR_PLUGIN_API },
    permissions: ["editor:read", "editor:write"],
    contexts: ["document", "session"],
    surfaces: ["desktop", "touch"],
    settings: [],
    ...overrides,
  };
}

async function pack(m: EditorPluginManifest, main: string): Promise<string> {
  return JSON.stringify({
    format: EDITOR_PLUGIN_PACKAGE_FORMAT,
    manifest: m,
    main,
    digest: await editorPluginDigest(m, main),
  });
}

/** An in-process stand-in for the sandbox: the real prelude and plugin code
 * run against a fake worker global and talk over a real MessageChannel. */
function fakeSandbox(main: string): EditorPluginTransport {
  const channel = new MessageChannel();
  const worker = new EventTarget() as EventTarget & Record<string, unknown>;
  const listeners: ((message: unknown) => void)[] = [];
  channel.port1.onmessage = (event) => {
    for (const listener of listeners) listener(event.data);
  };
  try {
    new Function(
      "self",
      `${EDITOR_PLUGIN_WORKER_PRELUDE}\n;(() => { const definePlugin = self.definePlugin;\n${main}\n})();`,
    )(worker);
  } catch (error) {
    queueMicrotask(() => {
      for (const listener of listeners) {
        listener({ t: "crash", error: String(error) });
      }
    });
  }
  const event = new MessageEvent("message", { ports: [channel.port2] });
  worker.dispatchEvent(event);
  return {
    post: (message) => channel.port1.postMessage(message),
    listen: (listener) => listeners.push(listener),
    terminate: () => {
      channel.port1.close();
      channel.port2.close();
    },
  };
}

function editor(
  initial = "b\na\nc",
  kind: "document" | "session" = "document",
) {
  let text = initial;
  let selection = { anchor: 0, head: text.length };
  let revision = 0;
  let alive = true;
  let composing = false;
  const port: EditorPort = {
    context: { kind, id: "doc", surface: "desktop" },
    read: () => ({ text, selection, revision }),
    replaceSelection: (value, expected) => {
      if (!alive || composing || expected.revision !== revision) return false;
      const from = Math.min(selection.anchor, selection.head);
      const to = Math.max(selection.anchor, selection.head);
      text = text.slice(0, from) + value + text.slice(to);
      selection = { anchor: from + value.length, head: from + value.length };
      revision++;
      return true;
    },
    reveal: () => {},
  };
  return {
    port,
    text: () => text,
    type: (value: string) => {
      text += value;
      revision++;
    },
    close: () => {
      alive = false;
    },
    compose: (value: boolean) => {
      composing = value;
    },
  };
}

function memory() {
  let stored: unknown = null;
  let failNext = false;
  return {
    persistence: {
      load: () =>
        Promise.resolve(stored === null ? null : structuredClone(stored)),
      save: (value: unknown) => {
        if (failNext) {
          failNext = false;
          return Promise.reject(new Error("disk full"));
        }
        stored = structuredClone(value);
        return Promise.resolve();
      },
    },
    stored: () => stored,
    failNextSave: () => {
      failNext = true;
    },
  };
}

function host(store = memory(), notices: string[] = []) {
  return createEditorPluginHost({
    persistence: store.persistence,
    sandbox: fakeSandbox,
    notice: (text) => notices.push(text),
    timeouts: { start: 300, command: 300, panel: 300, unload: 50 },
  });
}

const SORT = `definePlugin({
  async onload(ctx) {
    ctx.addCommand({ id: "sort", title: "Sort", toolbar: true, icon: "sort", async run(editor) {
      const lines = editor.read().text.split("\\n").sort();
      if (await editor.replaceSelection(lines.join("\\n"))) await ctx.saveData({ runs: 1 });
    }});
    ctx.addPanel({ id: "len", title: "Length", render: (doc) => [{ label: doc.text.length + " chars", offset: 0 }] });
  },
});`;

test("manifest validation is closed and versioned", async () => {
  parseEditorPluginManifest(manifest());
  for (
    const bad of [
      { ...manifest(), extra: true },
      manifest({ id: "cowboy-x" }),
      manifest({ version: "1.0" }),
      manifest({ permissions: ["network" as never] }),
      manifest({ contexts: [] }),
    ]
  ) {
    let threw = false;
    try {
      parseEditorPluginManifest(bad);
    } catch {
      threw = true;
    }
    assert(threw, JSON.stringify(bad));
  }
  const source = await pack(manifest(), SORT);
  await parseEditorPluginPackage(source);
  const tampered = JSON.parse(source);
  tampered.main += "\n// changed";
  await assertRejects(() => parseEditorPluginPackage(JSON.stringify(tampered)));
});

test("install runs commands, panels and data through version-bound edits", async () => {
  const store = memory();
  const plugins = host(store);
  const plan = await plugins.inspect(await pack(manifest(), SORT));
  assertEquals(plan.kind, "install");
  assertEquals(plan.newPermissions, ["editor:read", "editor:write"]);
  assertEquals((await plugins.install(plan)).ok, true);
  const view = plugins.getSnapshot().plugins[0]!;
  assertEquals(view.status, "running");
  assertEquals(view.commands.map((c) => [c.id, c.toolbar, c.icon]), [[
    "sort",
    true,
    "sort",
  ]]);
  const target = editor();
  await plugins.runCommand("sample", "sort", target.port);
  assertEquals(target.text(), "a\nb\nc");
  assertEquals(
    (await plugins.renderPanel("sample", "len", target.port)).map((i) =>
      i.label
    ),
    ["5 chars"],
  );
  const saved = store.stored() as { plugins: { data: unknown }[] };
  assertEquals(saved.plugins[0]!.data, { runs: 1 });
  await plugins.dispose();
});

test("edits are refused without permission, after typing or with an IME", async () => {
  const plugins = host();
  const readOnly = `definePlugin({ onload(ctx) {
    ctx.addCommand({ id: "w", title: "Write", async run(editor) { await editor.replaceSelection("x"); } });
  }});`;
  await plugins.install(
    await plugins.inspect(
      await pack(manifest({ permissions: ["editor:read"] }), readOnly),
    ),
  );
  const target = editor();
  await assertRejects(
    () => plugins.runCommand("sample", "w", target.port),
    Error,
    "editor:write",
  );
  assertEquals(target.text(), "b\na\nc");

  const raced = `definePlugin({ onload(ctx) {
    ctx.addCommand({ id: "late", title: "Late", async run(editor) {
      await new Promise((r) => setTimeout(r, 30));
      ctx.notice(String(await editor.replaceSelection("PLUGIN")));
    }});
  }});`;
  const notices: string[] = [];
  const second = host(memory(), notices);
  await second.install(
    await second.inspect(await pack(manifest({ id: "raced" }), raced)),
  );
  const typed = editor();
  const running = second.runCommand("raced", "late", typed.port);
  typed.type("!");
  await running;
  assertEquals(typed.text(), "b\na\nc!");
  const composing = editor();
  composing.compose(true);
  await second.runCommand("raced", "late", composing.port);
  assertEquals(composing.text(), "b\na\nc");
  assertEquals(notices.slice(-2), ["Sample: false", "Sample: false"]);
  await plugins.dispose();
  await second.dispose();
});

test("invocation tokens expire when the command settles", async () => {
  const leaky = `let saved; definePlugin({ onload(ctx) {
    ctx.addCommand({ id: "keep", title: "Keep", run(editor) { saved = editor; } });
    ctx.addCommand({ id: "use", title: "Use", async run() { ctx.notice(String(await saved.replaceSelection("late"))); } });
  }});`;
  const notices: string[] = [];
  const plugins = host(memory(), notices);
  await plugins.install(await plugins.inspect(await pack(manifest(), leaky)));
  const first = editor();
  await plugins.runCommand("sample", "keep", first.port);
  await plugins.runCommand("sample", "use", editor("other").port);
  assertEquals(first.text(), "b\na\nc");
  assertEquals(notices.at(-1), "Sample: false");
  await plugins.dispose();
});

test("a failing upgrade rolls back and other plugins keep running", async () => {
  const store = memory();
  const plugins = host(store);
  await plugins.install(await plugins.inspect(await pack(manifest(), SORT)));
  await plugins.install(
    await plugins.inspect(await pack(manifest({ id: "other" }), SORT)),
  );
  await plugins.updateSettings("sample", {});
  const broken = `definePlugin({ onload() { throw new Error("boom"); } });`;
  const plan = await plugins.inspect(
    await pack(manifest({ version: "1.1.0" }), broken),
  );
  assertEquals(plan.kind, "upgrade");
  const result = await plugins.install(plan);
  assertEquals(result.ok, false);
  const views = plugins.getSnapshot().plugins;
  assertEquals(
    views.map((v) => [v.manifest.id, v.manifest.version, v.status]),
    [
      ["sample", "1.0.0", "running"],
      ["other", "1.0.0", "running"],
    ],
  );
  const history = plugins.getSnapshot().history.map((h) => h.action);
  assertEquals(history.slice(-2), ["upgrade", "rollback"]);
  const target = editor();
  await plugins.runCommand("other", "sort", target.port);
  assertEquals(target.text(), "a\nb\nc");
  await plugins.dispose();
});

test("an unresponsive plugin is stopped and stays off after reload", async () => {
  const store = memory();
  const plugins = host(store);
  const hang = `definePlugin({ onload(ctx) {
    ctx.addCommand({ id: "hang", title: "Hang", run: () => new Promise(() => {}) });
  }});`;
  await plugins.install(await plugins.inspect(await pack(manifest(), hang)));
  await assertRejects(() =>
    plugins.runCommand("sample", "hang", editor().port)
  );
  await new Promise((r) => setTimeout(r, 20));
  const view = plugins.getSnapshot().plugins[0]!;
  assertEquals([view.enabled, view.status], [false, "failed"]);
  await plugins.dispose();
  const reloaded = host(store);
  await reloaded.ensureLoaded();
  assertEquals(reloaded.getSnapshot().plugins[0]!.status, "failed");
  await reloaded.setEnabled("sample", true);
  assertEquals(reloaded.getSnapshot().plugins[0]!.status, "running");
  await reloaded.dispose();
});

test("disable, settings and uninstall clean up everything", async () => {
  const store = memory();
  const plugins = host(store);
  const settingsPlugin = `definePlugin({ onload(ctx) {
    ctx.addCommand({ id: "say", title: "Say", async run(editor) { await editor.replaceSelection(ctx.settings.word); } });
  }});`;
  const m = manifest({
    settings: [{
      id: "word",
      type: "string",
      title: "Word",
      default: "hi",
      maxLength: 10,
    }],
  });
  await plugins.install(await plugins.inspect(await pack(m, settingsPlugin)));
  await plugins.updateSettings("sample", { word: "hello", unknown: 1 });
  await new Promise((r) => setTimeout(r, 10));
  const target = editor("x");
  await plugins.runCommand("sample", "say", target.port);
  assertEquals(target.text(), "hello");
  await plugins.setEnabled("sample", false);
  assertEquals(plugins.getSnapshot().plugins[0]!.commands, []);
  await assertRejects(() => plugins.runCommand("sample", "say", editor().port));
  await plugins.uninstall("sample");
  assertEquals(plugins.getSnapshot().plugins, []);
  assertEquals((store.stored() as { plugins: unknown[] }).plugins, []);
  await plugins.dispose();
});

test("a failed save leaves the installed state unchanged", async () => {
  const store = memory();
  const plugins = host(store);
  store.failNextSave();
  await assertRejects(async () =>
    await plugins.install(await plugins.inspect(await pack(manifest(), SORT)))
  );
  assertEquals(plugins.getSnapshot().plugins, []);
  await plugins.dispose();
});

test("the example package installs and sorts with its settings", async () => {
  const m = parseEditorPluginManifest(
    JSON.parse(await readFile(new URL("manifest.json", exampleDir), "utf8")),
  );
  const main = await readFile(new URL("main.js", exampleDir), "utf8");
  const plugins = host();
  await plugins.install(await plugins.inspect(await pack(m, main)));
  const view = plugins.getSnapshot().plugins[0]!;
  assertEquals(view.commands.filter((c) => c.toolbar).map((c) => c.id), [
    "sort-lines",
    "insert-timestamp",
  ]);
  await plugins.updateSettings("text-tools", { order: "desc" });
  await new Promise((r) => setTimeout(r, 10));
  const target = editor("b\na\nc");
  await plugins.runCommand("text-tools", "sort-lines", target.port);
  assertEquals(target.text(), "c\nb\na");
  const stats = await plugins.renderPanel(
    "text-tools",
    "statistics",
    editor("你好 world").port,
  );
  assertEquals(stats[0]!.label, "3 words");
  await plugins.dispose();
});
