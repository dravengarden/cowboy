import { documentNotice } from "../documents/DocumentNotifications";
import { productSessionSignal } from "../productSessionEnd";
import { type ProductCache, productSyncDatabase } from "../productSyncDatabase";
import { productSyncPrincipal } from "../productSyncIdentity";
import { createEditorPluginHost, type EditorPluginHost } from "./registry";
import { createSandboxTransport } from "./sandbox";

let host: EditorPluginHost | null = null;

/** The page's one editor plugin host, shared by every Draft and Session
 * editor. Installed plugins are device-local for the signed-in principal. */
export function editorPluginHost(): EditorPluginHost {
  if (host) return host;
  // Borrow the dataset cache on first use, not at construction: rendering a
  // toolbar after the product session ended must not throw.
  let cache: ProductCache<unknown> | null = null;
  const borrow = (): ProductCache<unknown> =>
    cache ??= productSyncDatabase.cache<unknown>({
      kind: "service",
      state: "editor-plugins",
    });
  const created = createEditorPluginHost({
    persistence: {
      load: async () => await borrow().load(),
      save: async (value) => await borrow().save(value),
    },
    sandbox: createSandboxTransport,
    notice: documentNotice,
  });
  // Sign-out ends this page's product lifetime; stop every plugin sandbox.
  productSessionSignal().addEventListener(
    "abort",
    () => void created.dispose(),
    {
      once: true,
    },
  );
  host = created;
  return created;
}

/** Start enabled plugins once a signed-in editor exists. Never blocks editing. */
export function loadEditorPlugins(): void {
  if (!productSyncPrincipal() || productSessionSignal().aborted) return;
  void editorPluginHost().ensureLoaded().catch(() => undefined);
}

/** Desktop command id shared by the palette and plugin toolbar buttons. */
export function editorPluginCommandId(plugin: string, command: string): string {
  return `plugin.${plugin}.${command}`;
}
