import { assertEquals, assertThrows } from "jsr:@std/assert";
import { createEditorExtensionRuntime } from "./runtime.ts";
import {
  documentOutline,
  expandEditorTemplate,
  templateExtension,
} from "./builtins.ts";
import type { EditorExtension, EditorPort } from "./contract.ts";

function editor(kind: "document" | "session") {
  let text = "中文 selected";
  let revision = 0;
  const port: EditorPort = {
    context: { kind, id: "sample", surface: "desktop" },
    read: () => ({
      text,
      selection: { anchor: 3, head: text.length },
      revision,
    }),
    replaceSelection: (value, expected) => {
      if (expected.revision !== revision) return false;
      text = text.slice(0, expected.selection.anchor) + value;
      revision++;
      return true;
    },
    reveal: () => undefined,
  };
  return port;
}

Deno.test("one extension runs through the same editor port in independent documents and sessions", async () => {
  for (const kind of ["document", "session"] as const) {
    const port = editor(kind);
    const runtime = createEditorExtensionRuntime(port);
    runtime.activate(
      templateExtension([{
        id: "example",
        title: "Example",
        text: "**{{selection}}**",
      }]),
    );
    assertEquals(runtime.commands()[0]?.id, "cowboy-templates:example");
    await runtime.commands()[0]!.run(port);
    assertEquals(port.read().text, "中文 **selected**");
    runtime.deactivate("cowboy-templates");
    assertEquals(runtime.commands(), []);
    runtime.dispose();
  }
});

Deno.test("failed activation unwinds all resources; disabled plugins lose their editor authority", () => {
  const port = editor("document");
  const runtime = createEditorExtensionRuntime(port);
  const calls: number[] = [];
  let held: EditorPort | undefined;
  const extension: EditorExtension = {
    id: "fixture-extension",
    version: "1.0.0",
    apiVersion: 1,
    title: "Fixture",
    description: "",
    contexts: ["document"],
    surfaces: ["desktop"],
    activate(scope) {
      held = scope.editor;
      scope.own(() => calls.push(1));
      scope.own(() => calls.push(2));
      scope.command({ id: "one", title: "One", run: () => undefined });
    },
  };
  runtime.activate(extension);
  const snapshot = held!.read();
  runtime.deactivate(extension.id);
  assertEquals(calls, [2, 1]);
  assertEquals(held!.replaceSelection("late", snapshot), false);
  assertThrows(() => held!.read());
  assertThrows(() =>
    runtime.activate({
      ...extension,
      activate(scope) {
        scope.own(() => calls.push(3));
        throw new Error("failed");
      },
    })
  );
  assertEquals(calls, [2, 1, 3]);
  assertEquals(runtime.commands().length, 0);
  runtime.dispose();
  runtime.dispose();
  assertThrows(() => runtime.activate(extension));
});

Deno.test("Markdown outline ignores fenced examples and template selection is literal", () => {
  assertEquals(
    documentOutline("# One\n```md\n# Not a heading\n```\n## 中文\n").map((
      row,
    ) => row.label),
    ["One", "中文"],
  );
  assertEquals(
    expandEditorTemplate(
      "{{date}} {{selection}}",
      "{{date}} $&",
      new Date(2026, 9, 4),
    ),
    "2026-10-04 {{date}} $&",
  );
});
