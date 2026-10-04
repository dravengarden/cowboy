/** Core editor extension API v1. Independent of Session, Machine and React.
 * Installable packages must be admitted by Cowboy's generic Plugin lifecycle;
 * this API does not download, install or evaluate untrusted package code. */
export const EDITOR_EXTENSION_API = 1 as const;

export interface EditorContext {
  readonly kind: "document" | "session";
  readonly id: string;
  readonly surface: "desktop" | "touch";
}
export interface EditorSnapshot {
  readonly text: string;
  readonly selection: { readonly anchor: number; readonly head: number };
  readonly revision: number;
}
export interface EditorPort {
  readonly context: EditorContext;
  read(): EditorSnapshot;
  /** Version-bound, undoable replacement. False means the user has continued
   * writing, an IME owns the field, or this editor's lifetime has ended. */
  replaceSelection(text: string, expected: EditorSnapshot): boolean;
  reveal(offset: number): void;
}
export interface EditorExtensionCommand {
  readonly id: string;
  readonly title: string;
  readonly description?: string;
  run(editor: EditorPort): void | Promise<void>;
}
export interface EditorPanelItem {
  readonly id: string;
  readonly label: string;
  readonly detail?: string;
  readonly offset?: number;
  readonly depth?: number;
}
export interface EditorExtensionPanel {
  readonly id: string;
  readonly title: string;
  read(snapshot: EditorSnapshot): readonly EditorPanelItem[];
}
export interface EditorExtensionScope {
  readonly editor: EditorPort;
  readonly signal: AbortSignal;
  command(command: EditorExtensionCommand): void;
  panel(panel: EditorExtensionPanel): void;
  /** Resources are disposed in reverse registration order on unload. */
  own(dispose: () => void): void;
}
export interface EditorExtension {
  readonly id: string;
  readonly version: string;
  readonly apiVersion: typeof EDITOR_EXTENSION_API;
  readonly title: string;
  readonly description: string;
  readonly contexts: readonly EditorContext["kind"][];
  readonly surfaces: readonly EditorContext["surface"][];
  activate(scope: EditorExtensionScope): void;
}
