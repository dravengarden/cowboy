import {
  EDITOR_EXTENSION_API,
  type EditorExtension,
  type EditorExtensionCommand,
  type EditorExtensionPanel,
  type EditorPort,
} from "./contract";

/** One resource scope per extension per actual editor lifetime. The core keeps
 * IME, Vim, history, storage and rendering ownership. No global DOM hooks are
 * handed to an extension and commands cannot replace the editor component. */
export function createEditorExtensionRuntime(editor: EditorPort) {
  const active = new Map<
    string,
    {
      dispose: () => void;
      commands: EditorExtensionCommand[];
      panels: EditorExtensionPanel[];
    }
  >();
  let closed = false;
  return {
    activate(extension: EditorExtension): void {
      if (closed) throw new Error("Editor extension host is closed");
      if (
        !/^[a-z][a-z0-9-]{1,63}$/.test(extension.id) ||
        !/^\d+\.\d+\.\d+$/.test(extension.version) ||
        extension.apiVersion !== EDITOR_EXTENSION_API
      ) throw new Error("Incompatible editor extension");
      if (active.has(extension.id)) return;
      if (
        !extension.contexts.includes(editor.context.kind) ||
        !extension.surfaces.includes(editor.context.surface)
      ) return;
      const lifetime = new AbortController();
      const cleanups: (() => void)[] = [];
      const commands: EditorExtensionCommand[] = [];
      const panels: EditorExtensionPanel[] = [];
      const claim = (id: string, values: readonly { id: string }[]): string => {
        if (!/^[a-z][a-z0-9-]{0,63}$/.test(id)) {
          throw new Error("Invalid editor contribution id");
        }
        const qualified = `${extension.id}:${id}`;
        if (values.some((v) => v.id === qualified)) {
          throw new Error("Duplicate editor contribution");
        }
        return qualified;
      };
      const dispose = (): void => {
        lifetime.abort();
        for (const cleanup of cleanups.reverse()) {
          try {
            cleanup();
          } catch { /* Dispose remaining owners. */ }
        }
        cleanups.length = 0;
      };
      try {
        const port: EditorPort = {
          context: editor.context,
          read: () => {
            if (lifetime.signal.aborted) {
              throw new Error("Extension is unloaded");
            }
            return editor.read();
          },
          replaceSelection: (text, expected) =>
            !lifetime.signal.aborted && editor.replaceSelection(text, expected),
          reveal: (offset) => {
            if (!lifetime.signal.aborted) editor.reveal(offset);
          },
        };
        extension.activate({
          editor: port,
          signal: lifetime.signal,
          command: (command) => {
            if (lifetime.signal.aborted) {
              throw new Error("Extension is unloaded");
            }
            commands.push({
              ...command,
              id: claim(command.id, commands),
              run: () => command.run(port),
            });
          },
          panel: (panel) => {
            if (lifetime.signal.aborted) {
              throw new Error("Extension is unloaded");
            }
            panels.push({ ...panel, id: claim(panel.id, panels) });
          },
          own: (cleanup) => {
            if (lifetime.signal.aborted) cleanup();
            else cleanups.push(cleanup);
          },
        });
        active.set(extension.id, { dispose, commands, panels });
      } catch (error) {
        dispose();
        throw error;
      }
    },
    deactivate(id: string): void {
      active.get(id)?.dispose();
      active.delete(id);
    },
    commands: (): readonly EditorExtensionCommand[] =>
      [...active.values()].flatMap((v) => v.commands),
    panels: (): readonly EditorExtensionPanel[] =>
      [...active.values()].flatMap((v) => v.panels),
    dispose(): void {
      if (closed) return;
      closed = true;
      for (const value of active.values()) value.dispose();
      active.clear();
    },
  };
}
