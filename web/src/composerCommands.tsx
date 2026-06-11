// The composer command registry — the Obsidian-style "toolbar extension"
// mechanism. EVERY composer content action is a command here (the inline marks,
// headings, lists, quote, code block, link, indent/outdent, highlight, AND the
// migrated `@` mention / `/` slash / attach). Toolbars render from this registry
// driven by a persisted ordered id list (composerToolbarConfig.ts), so adding a
// button is one entry and the user can curate the set. Send + agent-config stay
// bespoke (not content commands).
import type { ReactNode } from "react";
import {
  AlternateEmail,
  AttachFile,
  BorderColor,
  CheckBoxOutlined,
  Code,
  DataObject,
  FormatBold,
  FormatIndentDecrease,
  FormatIndentIncrease,
  FormatItalic,
  FormatListBulleted,
  FormatListNumbered,
  FormatQuote,
  InsertLink,
  Redo,
  StrikethroughS,
  Tag,
  Title,
  Undo,
} from "@mui/icons-material";
import type { ComposerEditorHandle } from "./ComposerEditor";

// The context a command runs against. The editor handle covers in-doc actions;
// `attach` drives the HOST file-picker (attachment isn't editor-only) — which is
// exactly why a command takes a context, not just the editor handle.
export interface ComposerCommandCtx {
  editor: ComposerEditorHandle;
  attach: () => void;
}

export interface ComposerCommand {
  id: string;
  icon: ReactNode;
  label: string;
  run: (ctx: ComposerCommandCtx) => void;
}

export const COMPOSER_COMMANDS: readonly ComposerCommand[] = [
  { id: "undo", icon: <Undo />, label: "Undo", run: (c): void => c.editor.undo() },
  { id: "redo", icon: <Redo />, label: "Redo", run: (c): void => c.editor.redo() },
  { id: "heading", icon: <Title />, label: "Toggle heading", run: (c): void => c.editor.cycleHeading() },
  { id: "bold", icon: <FormatBold />, label: "Toggle bold", run: (c): void => c.editor.toggleWrap("**") },
  { id: "italic", icon: <FormatItalic />, label: "Toggle italic", run: (c): void => c.editor.toggleWrap("*") },
  { id: "strikethrough", icon: <StrikethroughS />, label: "Toggle strikethrough", run: (c): void => c.editor.toggleWrap("~~") },
  { id: "highlight", icon: <BorderColor />, label: "Toggle highlight", run: (c): void => c.editor.toggleWrap("==") },
  { id: "code", icon: <Code />, label: "Toggle inline code", run: (c): void => c.editor.toggleWrap("`") },
  { id: "link", icon: <InsertLink />, label: "Insert link", run: (c): void => c.editor.insertLink() },
  { id: "bulletList", icon: <FormatListBulleted />, label: "Bulleted list", run: (c): void => c.editor.toggleLinePrefix("- ") },
  { id: "numberedList", icon: <FormatListNumbered />, label: "Numbered list", run: (c): void => c.editor.toggleLinePrefix("1. ") },
  { id: "checklist", icon: <CheckBoxOutlined />, label: "Checklist", run: (c): void => c.editor.toggleLinePrefix("- [ ] ") },
  { id: "quote", icon: <FormatQuote />, label: "Quote", run: (c): void => c.editor.toggleLinePrefix("> ") },
  { id: "codeBlock", icon: <DataObject />, label: "Code block", run: (c): void => c.editor.insertCodeBlock() },
  { id: "indent", icon: <FormatIndentIncrease />, label: "Indent", run: (c): void => c.editor.indent() },
  { id: "outdent", icon: <FormatIndentDecrease />, label: "Outdent", run: (c): void => c.editor.outdent() },
  { id: "mention", icon: <AlternateEmail />, label: "Mention file", run: (c): void => c.editor.insertTrigger("@") },
  { id: "slash", icon: <Tag />, label: "Slash command", run: (c): void => c.editor.insertTrigger("/") },
  { id: "attach", icon: <AttachFile />, label: "Insert attachment", run: (c): void => c.attach() },
];

export const COMPOSER_COMMANDS_BY_ID: Readonly<Record<string, ComposerCommand>> = Object
  .fromEntries(COMPOSER_COMMANDS.map((c) => [c.id, c]));
