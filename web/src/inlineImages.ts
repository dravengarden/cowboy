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
      widget.contentEditable = "false";
      // @replit/codemirror-vim walks into the DOM immediately after an EOL
      // cursor to borrow its font style. A block widget whose first descendant
      // is an empty <img> makes that walk end at `undefined`, so its cursor
      // measurement aborts and the Normal-mode block disappears on the text
      // line above the image. Keep a zero-size text node first: it is inert for
      // layout and accessibility, but gives the upstream measurement a safe
      // terminal node. Mark it non-selectable so physical iOS cannot park the
      // caret on this probe instead of the image source line.
      const probe = document.createElement("span");
      probe.setAttribute("aria-hidden", "true");
      probe.style.userSelect = "none";
      probe.textContent = "\u200b";
      widget.appendChild(probe);
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
        // Obsidian / atomic-editor: a block widget's default click lands on
        // the next line. Step back onto the source token line so the
        // markdown reveals and Return is a real text-line break.
        const pos = view.posAtDOM(widget);
        if (pos >= 0) {
          view.dispatch({
            selection: { anchor: Math.max(0, pos - 1) },
            scrollIntoView: false,
          });
        }
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
  // Ignore only pointer activation so a tap opens the popover. Keyboard,
  // IME, and line-break events stay on the source `.cm-line`.
  override ignoreEvent(event: Event): boolean {
    return event.type === "mousedown" || event.type === "click";
  }
}

// Obsidian / atomic-editor image-blocks: the token stays on a real `.cm-line`.
// The thumbnail hangs below (`widget`, `block: true`, `side: 1` at line.to).
// Hide the raw token only while the caret is on another line. Click lands
// on the source line so Return is a normal break in real text. Do not
// replace the source line out of flow and do not mark the field atomic.
const LONE_TOKEN_RE = /^\s*!\[([^\]]*)\]\(cowboy-att:([^)]+)\)\s*$/;
const IMG_TOKEN_RE = /!\[([^\]]*)\]\(cowboy-att:([^)]+)\)/g;

function lineIsActive(
  line: { from: number; to: number; number: number },
  sel: { empty: boolean; from: number; to: number; head: number },
  headLineNumber: number,
): boolean {
  return sel.empty
    ? headLineNumber === line.number
    : sel.from <= line.to && sel.to >= line.from;
}

function buildImageDecorations(state: EditorState): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const { doc } = state;
  const sel = state.selection.main;
  const headLineNumber = doc.lineAt(sel.head).number;
  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    const tokens: {
      from: number;
      to: number;
      id: string;
      alt: string;
    }[] = [];
    IMG_TOKEN_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = IMG_TOKEN_RE.exec(line.text)) !== null) {
      if (match[2] === undefined) continue;
      tokens.push({
        from: line.from + match.index,
        to: line.from + match.index + match[0].length,
        id: match[2],
        alt: match[1] ?? "",
      });
    }
    if (tokens.length === 0) continue;
    const active = lineIsActive(line, sel, headLineNumber);
    if (!active) {
      for (const token of tokens) {
        builder.add(token.from, token.to, Decoration.replace({}));
      }
    }
    for (const token of tokens) {
      const selected = !sel.empty && sel.from <= token.from && sel.to >= token.to;
      builder.add(
        line.to,
        line.to,
        Decoration.widget({
          widget: new InlineImageWidget(
            token.id,
            token.alt,
            selected,
            registry.get(token.id)?.previewUrl,
          ),
          block: true,
          side: 1,
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
  provide: (f) => EditorView.decorations.from(f),
});

function lastLineIsImageToken(text: string): boolean {
  const nl = text.lastIndexOf("\n");
  const last = nl === -1 ? text : text.slice(nl + 1);
  return last.length > 0 && LONE_TOKEN_RE.test(last);
}

/// Obsidian: a trailing image always has a following line, and deleting
/// that last newline puts it back.
export function ensureTrailingImageLine(text: string): string {
  return lastLineIsImageToken(text) ? `${text}\n` : text;
}

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
    userSelect: "none",
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
    IMG_TOKEN_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = IMG_TOKEN_RE.exec(line.text)) !== null) {
      if (match[2] !== id) continue;
      forgetInlineAttachment(id);
      const tokenFrom = line.from + match.index;
      const tokenTo = tokenFrom + match[0].length;
      const { from, to } = LONE_TOKEN_RE.test(line.text)
        ? imageDeletionRange(line.from, line.to, doc.length)
        : { from: tokenFrom, to: tokenTo };
      const current = view.state.selection.main;
      view.dispatch({
        changes: { from, to },
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
