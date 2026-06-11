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
  CommentOutlined,
  DataObject,
  FormatBold,
  FormatClear,
  FormatIndentDecrease,
  FormatIndentIncrease,
  FormatItalic,
  FormatListBulleted,
  FormatListNumbered,
  FormatQuote,
  Functions,
  InsertLink,
  Redo,
  StrikethroughS,
  Tag,
  TaskAlt,
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

// MUI ships no H1–H6 glyphs, so the discrete "Set as heading N" commands use a
// compact monospace `H{n}` badge sized to match the toolbar's icon metrics —
// distinguishable on the toolbar (where only the icon shows), unlike reusing one
// `Title` icon for all six.
function HeadingBadge({ n }: { n: number }): ReactNode {
  return (
    <span
      style={{
        fontFamily: "ui-monospace, monospace",
        fontWeight: 700,
        fontSize: "0.8rem",
        lineHeight: 1,
        letterSpacing: "-0.5px",
      }}
    >
      {`H${String(n)}`}
    </span>
  );
}

export const COMPOSER_COMMANDS: readonly ComposerCommand[] = [
  { id: "undo", icon: <Undo />, label: "Undo", run: (c): void => c.editor.undo() },
  { id: "redo", icon: <Redo />, label: "Redo", run: (c): void => c.editor.redo() },
  { id: "heading", icon: <Title />, label: "Toggle heading", run: (c): void => c.editor.cycleHeading() },
  { id: "heading1", icon: <HeadingBadge n={1} />, label: "Set as heading 1", run: (c): void => c.editor.setHeading(1) },
  { id: "heading2", icon: <HeadingBadge n={2} />, label: "Set as heading 2", run: (c): void => c.editor.setHeading(2) },
  { id: "heading3", icon: <HeadingBadge n={3} />, label: "Set as heading 3", run: (c): void => c.editor.setHeading(3) },
  { id: "heading4", icon: <HeadingBadge n={4} />, label: "Set as heading 4", run: (c): void => c.editor.setHeading(4) },
  { id: "heading5", icon: <HeadingBadge n={5} />, label: "Set as heading 5", run: (c): void => c.editor.setHeading(5) },
  { id: "heading6", icon: <HeadingBadge n={6} />, label: "Set as heading 6", run: (c): void => c.editor.setHeading(6) },
  { id: "removeHeading", icon: <FormatClear />, label: "Remove heading", run: (c): void => c.editor.setHeading(0) },
  { id: "bold", icon: <FormatBold />, label: "Toggle bold", run: (c): void => c.editor.toggleWrap("**") },
  { id: "italic", icon: <FormatItalic />, label: "Toggle italic", run: (c): void => c.editor.toggleWrap("*") },
  { id: "strikethrough", icon: <StrikethroughS />, label: "Toggle strikethrough", run: (c): void => c.editor.toggleWrap("~~") },
  { id: "highlight", icon: <BorderColor />, label: "Toggle highlight", run: (c): void => c.editor.toggleWrap("==") },
  { id: "inlineMath", icon: <Functions />, label: "Toggle inline math", run: (c): void => c.editor.toggleWrap("$") },
  { id: "comment", icon: <CommentOutlined />, label: "Toggle comment", run: (c): void => c.editor.toggleWrap("%%") },
  { id: "code", icon: <Code />, label: "Toggle inline code", run: (c): void => c.editor.toggleWrap("`") },
  { id: "link", icon: <InsertLink />, label: "Insert link", run: (c): void => c.editor.insertLink() },
  { id: "bulletList", icon: <FormatListBulleted />, label: "Bulleted list", run: (c): void => c.editor.toggleLinePrefix("- ") },
  { id: "numberedList", icon: <FormatListNumbered />, label: "Numbered list", run: (c): void => c.editor.toggleLinePrefix("1. ") },
  { id: "checklist", icon: <CheckBoxOutlined />, label: "Checklist", run: (c): void => c.editor.toggleLinePrefix("- [ ] ") },
  { id: "toggleCheckbox", icon: <TaskAlt />, label: "Toggle checkbox status", run: (c): void => c.editor.toggleCheckbox() },
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
