import type { EditorExtension, EditorPanelItem } from "./contract";

export interface EditorTemplate {
  readonly id: string;
  readonly title: string;
  readonly text: string;
}
export const DEFAULT_EDITOR_TEMPLATES: readonly EditorTemplate[] = [
  {
    id: "plan",
    title: "Plan",
    text:
      "## Goal\n\n{{selection}}\n\n## Steps\n\n- [ ] \n\n## Verification\n\n",
  },
  { id: "note", title: "Dated note", text: "## {{date}}\n\n{{selection}}\n" },
];

export function expandEditorTemplate(
  template: string,
  selection: string,
  date: Date,
): string {
  const localDate = `${date.getFullYear()}-${
    String(date.getMonth() + 1).padStart(2, "0")
  }-${String(date.getDate()).padStart(2, "0")}`;
  return template.replace(
    /\{\{(selection|date)\}\}/g,
    (_, token: string) => token === "selection" ? selection : localDate,
  );
}

export function templateExtension(
  templates: readonly EditorTemplate[],
): EditorExtension {
  return {
    id: "cowboy-templates",
    version: "1.0.0",
    apiVersion: 1,
    title: "Templates",
    description:
      "Insert your own reusable Markdown, with selection and date placeholders.",
    contexts: ["document", "session"],
    surfaces: ["desktop", "touch"],
    activate: ({ command }) => {
      for (const template of templates) {
        command({
          id: template.id,
          title: template.title,
          run: (editor) => {
            const snapshot = editor.read();
            const { anchor, head } = snapshot.selection;
            const selection = snapshot.text.slice(
              Math.min(anchor, head),
              Math.max(anchor, head),
            );
            if (
              !editor.replaceSelection(
                expandEditorTemplate(template.text, selection, new Date()),
                snapshot,
              )
            ) {
              throw new Error("Finish composing, then run the template again.");
            }
          },
        });
      }
    },
  };
}

export function documentOutline(text: string): readonly EditorPanelItem[] {
  const items: EditorPanelItem[] = [];
  let offset = 0;
  let fence: string | null = null;
  for (const line of text.split("\n")) {
    const marker = /^\s{0,3}(`{3,}|~{3,})/.exec(line)?.[1];
    if (marker) {
      if (!fence) fence = marker;
      else if (marker[0] === fence[0] && marker.length >= fence.length) {
        fence = null;
      }
    } else if (!fence) {
      const heading = /^ {0,3}(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
      if (heading) {
        items.push({
          id: `heading-${offset}`,
          label: heading[2]!,
          depth: heading[1]!.length - 1,
          offset,
        });
      }
    }
    offset += line.length + 1;
  }
  return items;
}

export const outlineExtension: EditorExtension = {
  id: "cowboy-outline",
  version: "1.0.0",
  apiVersion: 1,
  title: "Outline & statistics",
  description:
    "Navigate headings and inspect document length. Works in every editor.",
  contexts: ["document", "session"],
  surfaces: ["desktop", "touch"],
  activate: ({ panel }) => {
    panel({
      id: "outline",
      title: "Outline",
      read: ({ text }) => documentOutline(text),
    });
    panel({
      id: "statistics",
      title: "Statistics",
      read: ({ text }) => [
        {
          id: "characters",
          label: `${[...text].length.toLocaleString()} characters`,
        },
        {
          id: "lines",
          label: `${text.split("\n").length.toLocaleString()} lines`,
        },
      ],
    });
  },
};
