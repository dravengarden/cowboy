import type {
  Completion,
  CompletionContext,
  CompletionResult,
} from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import type { AvailableCommand } from "./protocol";

// `@path` file references. Async fuzzy search against the daemon — the same
// gitignore-aware `/api/sessions/{id}/files` endpoint the old Popper picker
// used. Triggers on an `@` at start-of-input or after whitespace; the server
// already ranks, so `filter: false` keeps its order. (Plan Step 8.)
export function fileCompletionSource(
  sessionId: string,
): (context: CompletionContext) => Promise<CompletionResult | null> {
  return async (
    context: CompletionContext,
  ): Promise<CompletionResult | null> => {
    const match = context.matchBefore(/(?:^|\s)@\S*/);
    if (!match) return null;
    const atOffset = match.text.indexOf("@");
    if (atOffset < 0) return null;
    const from = match.from + atOffset;
    const query = match.text.slice(atOffset + 1);
    const url = `/api/sessions/${encodeURIComponent(sessionId)}/files?q=${
      encodeURIComponent(query)
    }&limit=20`;
    let files: string[] = [];
    try {
      const r = await fetch(url);
      if (!r.ok) return null;
      const d = (await r.json()) as { files?: string[] };
      files = Array.isArray(d.files) ? d.files : [];
    } catch {
      return null; // transient — drop silently
    }
    // CM discards results for a superseded query, but bail early if the doc
    // already moved on.
    if (context.aborted || files.length === 0) return null;
    return {
      from,
      to: match.to,
      filter: false,
      options: files.map((path) => {
        const slash = path.lastIndexOf("/");
        const dir = slash >= 0 ? path.slice(0, slash) : "";
        return {
          label: `@${path}`,
          displayLabel: slash >= 0 ? path.slice(slash + 1) : path,
          apply: `@${path} `,
          type: "text",
          ...(dir ? { detail: dir } : {}),
        };
      }),
    };
  };
}

// `/skill` command picker. Mirrors the old first-position-only rule: only fires
// when the slash token starts at the very beginning of the input. Options come
// from the agent-advertised `availableCommands` (read fresh via the thunk so a
// late `available_commands_update` is reflected). (Plan Step 9.)
export function slashCompletionSource(
  commands: () => AvailableCommand[],
  onSelect?: (command: string) => void,
): (context: CompletionContext) => CompletionResult | null {
  return (context: CompletionContext): CompletionResult | null => {
    const match = context.matchBefore(/^\/\S*/);
    if (!match || match.from !== 0) return null;
    const cmds = commands();
    if (cmds.length === 0) return null;
    return {
      from: match.from,
      to: match.to,
      options: cmds.map((c) => ({
        label: `/${c.name}`,
        detail: c.description,
        apply: (
          view: EditorView,
          _completion: Completion,
          from: number,
          to: number,
        ): void => {
          const insert = `/${c.name} `;
          onSelect?.(c.name);
          view.dispatch({
            changes: { from, to, insert },
            selection: { anchor: from + insert.length },
            userEvent: "input.complete",
          });
        },
        type: "keyword",
      })),
    };
  };
}
