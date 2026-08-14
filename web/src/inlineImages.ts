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
import { EditorState, RangeSetBuilder, StateEffect, StateField } from "@codemirror/state";
import type { Attachment } from "./attachments";
import { openLightbox } from "./ResourceLightbox";
import {
  imageDeletionRange,
  inlineImageInsertion,
  mapImageDeletionPosition,
} from "./inlineImageSelection";

const registry = new Map<string, Attachment>();

// Bridge from the (non-React) image widget to the host's selection popover. The
// host (Composer) registers a handler; a tap on an inline image calls it with the
// image id + its DOM node (to anchor the popover). Falls back to opening the
// lightbox directly when no host has registered (e.g. a stray mount). Single
// module-level slot is fine: only one composer surface is mounted at a time.
let imageTapHandler:
  | ((id: string, el: HTMLElement, x: number, y: number) => void)
  | null = null;
export function setImageTapHandler(
  fn: ((id: string, el: HTMLElement, x: number, y: number) => void) | null,
): void {
  imageTapHandler = fn;
}

/// Register an attachment's bytes/preview so its inline token renders. Call this
/// BEFORE inserting the token (insertImageToken does it for you), and re-seed the
/// whole set when restoring a draft so its tokens render on mount.
export function registerInlineAttachment(a: Attachment): void {
  registry.set(a.id, a);
}
export function seedInlineAttachments(list: readonly Attachment[]): void {
  for (const a of list) registry.set(a.id, a);
}
/// Look up a registered inline attachment by id. The registry holds EVERY inline
/// image across all editor surfaces (main composer, draft/queue edit, the
/// fullscreen overlay — each seeds/registers into it), so this resolves an image
/// regardless of which editor's local `attachments` array it lives in. Use it for
/// the tap popover's Preview so the lightbox opens even in the expanded/overlay
/// editor (where the singleton tap handler's `attachments` is the wrong array).
export function getInlineAttachment(id: string): Attachment | undefined {
  return registry.get(id);
}
export function forgetInlineAttachment(id: string): void {
  registry.delete(id);
}

export const refreshInlineImages = StateEffect.define<null>();

class InlineImageWidget extends WidgetType {
  constructor(
    private readonly id: string,
    private readonly name: string,
    // True when the doc selection covers this image's block line (Backspace-armed
    // delete, or a range select) — draws the selection ring.
    private readonly selected: boolean,
    private readonly previewUrl: string | undefined,
  ) {
    super();
  }
  override eq(other: InlineImageWidget): boolean {
    return other.id === this.id && other.name === this.name &&
      other.selected === this.selected && other.previewUrl === this.previewUrl;
  }
  override toDOM(view: EditorView): HTMLElement {
    const att = registry.get(this.id);
    if (att?.previewUrl !== undefined && att.isImage) {
      const widget = document.createElement("span");
      widget.className = "cm-inline-image-widget";
      // @replit/codemirror-vim walks into the DOM immediately after an EOL
      // cursor to borrow its font style. A block widget whose first descendant
      // is an empty <img> makes that walk end at `undefined`, so its cursor
      // measurement aborts and the Normal-mode block disappears on the text
      // line above the image. Keep a zero-size text node first: it is inert for
      // layout and accessibility, but gives the upstream measurement a safe
      // terminal node. The document selection itself never enters this widget.
      widget.appendChild(document.createTextNode("\u200b"));
      const img = document.createElement("img");
      img.className = this.selected
        ? "cm-inline-image cm-inline-image-selected"
        : "cm-inline-image";
      img.src = att.previewUrl;
      img.alt = att.name;
      img.draggable = false;
      // Keep the caret visible after a paste: a tall inline image only gets its
      // real height once the <img> LOADS, so a scrollIntoView at insert time would
      // measure it as 0 and under-scroll — leaving the just-landed caret below the
      // image, hidden behind the keyboard. Re-scroll once it lays out. Gated on
      // focus so seeding a draft's images on mount (editor unfocused) doesn't yank
      // the view; `y: "nearest"` only scrolls if the caret is actually off-screen.
      img.addEventListener("load", () => {
        if (!view.hasFocus) return;
        view.dispatch({
          effects: EditorView.scrollIntoView(view.state.selection.main.head, {
            y: "nearest",
          }),
        });
      });
      const id = this.id;
      // mousedown (not click): beat CM's own pointer handling and stop the press
      // from moving the caret into the atomic widget. Hand off to the host's
      // selection popover (preview / delete); fall back to the lightbox directly.
      img.addEventListener("mousedown", (e) => {
        e.preventDefault();
        e.stopPropagation();
        // Hand the tap point to the host so the popover opens at the finger, not
        // anchored to this (possibly tall) image's bottom edge.
        if (imageTapHandler !== null) imageTapHandler(id, img, e.clientX, e.clientY);
        else openLightbox([att], 0);
      });
      widget.appendChild(img);
      return widget;
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
// its own line, and the whole line is replaced by a block widget. Physical
// v1247 showed that nesting the same widget in a `.cm-line` does not give
// WKWebView a measurable empty-line caret. Keep the proven block replacement
// and repair the empty-line caret with a document-neutral DOM anchor instead.
const LONE_TOKEN_RE = /^\s*!\[([^\]]*)\]\(cowboy-att:([^)]+)\)\s*$/;

function buildImageDecorations(state: EditorState): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const { doc } = state;
  const sel = state.selection.main;
  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    const m = LONE_TOKEN_RE.exec(line.text);
    if (m?.[2] !== undefined) {
      const selected = !sel.empty && sel.from <= line.from && sel.to >= line.to;
      builder.add(
        line.from,
        line.to,
        Decoration.replace({
          widget: new InlineImageWidget(
            m[2],
            m[1] ?? "",
            selected,
            registry.get(m[2])?.previewUrl,
          ),
          block: true,
        }),
      );
    }
  }
  return builder.finish();
}

export const inlineImageField = StateField.define<DecorationSet>({
  create: (state) => buildImageDecorations(state),
  update: (value, tr) =>
    tr.docChanged || tr.selection || tr.effects.some((effect) => effect.is(refreshInlineImages))
      ? buildImageDecorations(tr.state)
      : value,
  provide: (f) => [
    EditorView.decorations.from(f),
    EditorView.atomicRanges.of(
      (view) => view.state.field(f, false) ?? Decoration.none,
    ),
  ],
});

/// True when `text`'s LAST line is a lone block-image token (with no empty line
/// after it). A block-image decoration replaces its whole line and is atomic, so
/// as the doc's final line it traps the caret — you can't get below it to add a
/// line or keep typing. The two guards below keep a trailing newline so there's
/// always a landing line under a trailing image.
function lastLineIsBlockImage(text: string): boolean {
  const nl = text.lastIndexOf("\n");
  const last = nl === -1 ? text : text.slice(nl + 1);
  return last.length > 0 && LONE_TOKEN_RE.test(last);
}

/// Normalise a SEED value: append a newline when it ends with a block-image
/// token, so a restored draft / handed-off text never opens with the image
/// trapped as the last line. Idempotent; the extra line is whitespace, trimmed
/// back off on send (saveDraft/submit use trimEnd).
export function ensureTrailingImageLine(text: string): string {
  return lastLineIsBlockImage(text) ? `${text}\n` : text;
}

/// Keep a block image from ever being the doc's LAST line during editing. After
/// any change that leaves a trailing image token (e.g. the user backspaced the
/// empty line under it), append a newline so the caret can still land below it
/// ("图片在最后一行,无法开启新的一行"). Runs in the same transaction — no loop (the
/// appended newline makes the last line empty, so it won't re-match).
export const inlineImageTrailingLine = EditorState.transactionFilter.of((tr) => {
  if (!tr.docChanged) return tr;
  const last = tr.newDoc.line(tr.newDoc.lines);
  return last.length > 0 && LONE_TOKEN_RE.test(last.text)
    ? [tr, { changes: { from: tr.newDoc.length, insert: "\n" }, sequential: true }]
    : tr;
});

/// Thumbnail sizing/look. Kept here so any host of `inlineImagePlugin` gets it.
export const inlineImageTheme = EditorView.theme({
  ".cm-inline-image-widget": {
    display: "block",
    fontSize: "0",
    lineHeight: "0",
  },
  ".cm-inline-image": {
    display: "block",
    // A SMALL tap-to-open thumbnail, not a full inline preview: the token is a
    // placeholder you tap to open the lightbox (full image), so it only needs to
    // be big enough to recognize + tap — a large inline image otherwise ate most
    // of the composer height. Aspect ratio preserved (no crop); the 80px cap +
    // 200px width keep it compact while staying a comfortable tap target.
    maxWidth: "min(100%, 200px)",
    maxHeight: "80px",
    width: "auto",
    height: "auto",
    borderRadius: "8px",
    margin: "4px 0",
    cursor: "pointer",
    // A theme-aware hairline guarantees the thumbnail has a visible EDGE on BOTH
    // themes. The old black drop-shadow vanished on the dark surface — a pasted
    // dark screenshot blended into the dark panel with no boundary.
    // `--atomic-editor-border` is the MUI divider (cmTheme): a faint dark line on
    // light, a faint light line on dark. Keep a soft shadow for a touch of depth
    // (negligible on dark, a light lift on light).
    border: "1px solid var(--atomic-editor-border, rgba(128, 128, 128, 0.3))",
    boxShadow: "0 1px 4px rgba(0, 0, 0, 0.12)",
  },
  // Selection ring while the host's preview/delete popover is open for this image.
  // INSET (outline-offset negative) so the ring is drawn just inside the image's
  // own edges — an OUTSET ring (positive offset) gets clipped on the sides by the
  // editor's overflow container ("选中边框残/缺"). Modern WebKit rounds the outline
  // to the image's border-radius, so it tracks the rounded corners.
  ".cm-inline-image.cm-inline-image-selected": {
    outline: "2.5px solid var(--atomic-editor-accent-bright, #a78bfa)",
    outlineOffset: "-3px",
  },
});

/// Insert an image at the caret as an inline token (registers its bytes first so
/// the decoration renders immediately). A trailing space keeps the atomic token
/// from gluing to following text and lands the caret after it to keep typing.
export function insertImageToken(view: EditorView, a: Attachment): void {
  registerInlineAttachment(a);
  const { state } = view;
  const selection = state.selection.main;
  const edit = inlineImageInsertion(
    state.doc.toString(),
    selection.anchor,
    selection.head,
    [a],
  );
  view.dispatch({
    changes: { from: edit.from, to: edit.to, insert: edit.insert },
    selection: { anchor: edit.caret },
    // Best-effort immediate scroll (the image is still height-0 here); the
    // widget's load handler re-scrolls once it lays out for the real position.
    scrollIntoView: true,
  });
  view.focus();
}

/// Remove a specific image (by id) from the doc — the popover's Delete action.
/// Finds the lone-token line, deletes it plus its surrounding line breaks, and
/// forgets the bytes. No-op if the token isn't present.
export function removeImageTokenById(view: EditorView, id: string): void {
  const { doc } = view.state;
  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    const m = LONE_TOKEN_RE.exec(line.text);
    if (m?.[2] === id) {
      forgetInlineAttachment(id);
      const { from, to } = imageDeletionRange(line.from, line.to, doc.length);
      const current = view.state.selection.main;
      view.dispatch({
        changes: { from, to },
        // Preserve the active logical caret through the removed block. The old
        // code always forced the image line's start and left the leading layout
        // newline behind, turning deletion into a jump across a stale line.
        selection: {
          anchor: mapImageDeletionPosition(current.anchor, from, to),
          head: mapImageDeletionPosition(current.head, from, to),
        },
      });
      return;
    }
  }
}

// `\n?` token `\n?` — a whole image block line (token + its surrounding newlines).
const IMG_BLOCK_RE = /\n?!\[[^\]]*\]\(cowboy-att:([^)]+)\)\n?$/;

/// TWO-STAGE Backspace for inline images, so a stray keypress can't wipe a picture:
///   • caret right after an image block → SELECT it (don't delete) + ring it;
///   • that image block already selected → DELETE it (token + newlines) + forget.
/// Returns false otherwise so the normal / @-token Backspace still runs. Wire it
/// BEFORE deleteTokenBackward.
export function deleteImageTokenBackward(view: EditorView): boolean {
  const { state } = view;
  const range = state.selection.main;

  // Stage 2: an image block is the current selection → delete it.
  if (!range.empty) {
    const sel = state.sliceDoc(range.from, range.to);
    const m = /^\n?!\[[^\]]*\]\(cowboy-att:([^)]+)\)\n?$/.exec(sel);
    if (m === null) return false; // a normal selection → default Backspace
    if (m[1] !== undefined) forgetInlineAttachment(m[1]);
    view.dispatch({
      changes: { from: range.from, to: range.to },
      selection: { anchor: range.from },
    });
    return true;
  }

  // Stage 1: caret just after an image block → select it (a second Backspace, or
  // the popover's Delete, confirms). The selection drives the ring via the field.
  const head = range.head;
  const before = state.doc.sliceString(Math.max(0, head - 600), head);
  const m = IMG_BLOCK_RE.exec(before);
  if (m === null) return false;
  const from = head - m[0].length;
  view.dispatch({ selection: { anchor: from, head } });
  return true;
}
