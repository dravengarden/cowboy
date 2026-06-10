// Obsidian/Zed-style INLINE images for the composer. A pasted/attached image is
// written into the literal doc as a token `![name](cowboy-att:<id>)` at the caret
// (see attachments.ts `IMG_TOKEN_RE`); this module renders that token as an
// atomic, read-only thumbnail widget right there in the text flow, click → the
// shared lightbox. Atomic + non-editable = the SAME class as the `@`-token chip
// (fileTokenWidget.ts), so it is IME-safe — it is NOT a contenteditable widget
// (the only IME hazard, table-widget.ts, stays excluded; see mdlive/PITFALLS.md).
//
// The bytes/preview never enter the doc. The widget resolves `<id>` → its
// `Attachment` from a module-level registry that the host (Composer) populates
// BEFORE dispatching the token insert, so the decoration — rebuilt on the
// docChange the insert causes — finds the preview synchronously, with no React
// render race. The doc text + the host's `attachments[]` remain the real sources
// of truth; this registry is only a render-time lookup.
import {
  Decoration,
  type DecorationSet,
  EditorView,
  WidgetType,
} from "@codemirror/view";
import { type EditorState, RangeSetBuilder, StateField } from "@codemirror/state";
import { type Attachment, IMG_TOKEN_RE } from "./attachments";
import { openLightbox } from "./ResourceLightbox";

const registry = new Map<string, Attachment>();

/// Register an attachment's bytes/preview so its inline token renders. Call this
/// BEFORE inserting the token (insertImageToken does it for you), and re-seed the
/// whole set when restoring a draft so its tokens render on mount.
export function registerInlineAttachment(a: Attachment): void {
  registry.set(a.id, a);
}
export function seedInlineAttachments(list: readonly Attachment[]): void {
  for (const a of list) registry.set(a.id, a);
}
export function forgetInlineAttachment(id: string): void {
  registry.delete(id);
}

/// Turn the composer's `![name](cowboy-att:id)` tokens into REAL markdown images
/// (`![name](previewUrl)`) for any read-only display (transcript bubble, queue /
/// draft previews) so MarkdownImpl renders the actual picture + lightbox instead
/// of a broken `cowboy-att:` src. A token whose bytes aren't registered on this
/// device (e.g. synced history after a reload) is dropped — clean text, no broken
/// image. Use this anywhere a sent/queued/draft message's text is rendered.
export function inlineTokensToMarkdown(text: string): string {
  return text.replace(IMG_TOKEN_RE, (_full, id: string) => {
    const att = registry.get(id);
    return att?.previewUrl !== undefined && att.isImage
      ? `![${att.name}](${att.previewUrl})`
      : "";
  });
}

class InlineImageWidget extends WidgetType {
  constructor(
    private readonly id: string,
    private readonly name: string,
  ) {
    super();
  }
  override eq(other: InlineImageWidget): boolean {
    return other.id === this.id && other.name === this.name;
  }
  override toDOM(): HTMLElement {
    const att = registry.get(this.id);
    if (att?.previewUrl !== undefined && att.isImage) {
      const img = document.createElement("img");
      img.className = "cm-inline-image";
      img.src = att.previewUrl;
      img.alt = att.name;
      img.draggable = false;
      // mousedown (not click): beat CM's own pointer handling and stop the press
      // from moving the caret into the atomic widget; open the shared lightbox.
      img.addEventListener("mousedown", (e) => {
        e.preventDefault();
        e.stopPropagation();
        openLightbox([att], 0);
      });
      return img;
    }
    // Bytes not locally available (e.g. a synced token with no blob on this
    // device) — a compact chip keeps the token visible + deletable.
    const chip = document.createElement("span");
    chip.className = "cm-token-chip";
    chip.textContent = `🖼 ${this.name || "image"}`;
    return chip;
  }
  // Self-contained widget: the editor ignores pointer events on it (so a tap
  // opens the lightbox instead of placing the caret); arrow-key motion is handled
  // by the atomicRanges provider below.
  override ignoreEvent(): boolean {
    return true;
  }
}

// One image == one BLOCK line (Obsidian-style): the token is inserted alone on
// its own line, and the whole line is replaced by a block widget. Block (not
// inline) so a tall image never grows the surrounding line box — the caret on the
// text lines above/below stays a normal text-height bar (an inline image made the
// caret as tall as the picture: "光标好丑"). Lines that AREN'T a lone token are
// left untouched (a token accidentally mid-line just stays literal — rare).
const LONE_TOKEN_RE = /^\s*!\[([^\]]*)\]\(cowboy-att:([^)]+)\)\s*$/;

function buildImageDecorations(state: EditorState): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const { doc } = state;
  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    const m = LONE_TOKEN_RE.exec(line.text);
    if (m?.[2] !== undefined) {
      builder.add(
        line.from,
        line.to,
        Decoration.replace({
          widget: new InlineImageWidget(m[2], m[1] ?? ""),
          block: true,
        }),
      );
    }
  }
  return builder.finish();
}

/// The inline-image decoration — render `cowboy-att:` tokens as atomic, BLOCK
/// thumbnails (one image per line). MUST be a StateField, not a ViewPlugin: CM6
/// throws "Block decorations may not be specified via plugins" for `block: true`
/// decorations from a plugin (that regressed image paste in v171). Rebuilds on any
/// docChange (the composer is small, so a full-doc scan is cheap); provides both
/// the decorations and the atomic ranges (arrow keys skip an image as one unit).
export const inlineImageField = StateField.define<DecorationSet>({
  create: (state) => buildImageDecorations(state),
  update: (value, tr) => (tr.docChanged ? buildImageDecorations(tr.state) : value),
  provide: (f) => [
    EditorView.decorations.from(f),
    EditorView.atomicRanges.of(
      (view) => view.state.field(f, false) ?? Decoration.none,
    ),
  ],
});

/// Thumbnail sizing/look. Kept here so any host of `inlineImagePlugin` gets it.
export const inlineImageTheme = EditorView.theme({
  ".cm-inline-image": {
    display: "block",
    maxWidth: "min(100%, 360px)",
    maxHeight: "320px",
    width: "auto",
    height: "auto",
    borderRadius: "10px",
    margin: "4px 0",
    cursor: "zoom-in",
    boxShadow: "0 1px 6px rgba(0,0,0,0.22)",
  },
});

/// Insert an image at the caret as an inline token (registers its bytes first so
/// the decoration renders immediately). A trailing space keeps the atomic token
/// from gluing to following text and lands the caret after it to keep typing.
export function insertImageToken(view: EditorView, a: Attachment): void {
  registerInlineAttachment(a);
  const { state } = view;
  const pos = state.selection.main.head;
  // Put the image alone on its OWN line so it block-renders (see the decoration).
  // Lead with a newline unless we're already at line start; always trail with one,
  // landing the caret on a fresh line below the image, ready to keep typing.
  const atLineStart = pos === state.doc.lineAt(pos).from;
  const insert = `${atLineStart ? "" : "\n"}![${a.name}](cowboy-att:${a.id})\n`;
  view.dispatch({
    changes: { from: pos, insert },
    selection: { anchor: pos + insert.length },
  });
  view.focus();
}

/// Backspace that removes a whole inline-image token in one press (caret just
/// after `…(cowboy-att:…)` or its trailing space). Returns false otherwise so the
/// normal / @-token Backspace still runs. Wire it BEFORE deleteTokenBackward.
export function deleteImageTokenBackward(view: EditorView): boolean {
  const { state } = view;
  const range = state.selection.main;
  if (!range.empty) return false;
  const head = range.head;
  const before = state.doc.sliceString(Math.max(0, head - 600), head);
  // The token lives alone on its own line (insertImageToken). Consume the token
  // plus the surrounding newlines so one Backspace removes the whole image line
  // and rejoins the text around it.
  const m = /\n?!\[[^\]]*\]\(cowboy-att:([^)]+)\)\n?$/.exec(before);
  if (m === null) return false;
  const from = head - m[0].length;
  if (m[1] !== undefined) forgetInlineAttachment(m[1]);
  view.dispatch({ changes: { from, to: head }, selection: { anchor: from } });
  return true;
}
