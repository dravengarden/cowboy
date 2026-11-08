// Cowboy Widgets v1: the agent-authored structured UI document.
//
// Design: docs/widgets.md. An agent emits this JSON through Cowboy MCP
// (`widgets.publish` / `widgets.update`) or inside a ```cowboy-widget fence.
// Cowboy validates it at both trust boundaries (Controller in Rust, Web in
// TypeScript, shared fixtures) and renders it with product MUI components.
// Nothing in a document is executed: no markup, no scripts, no styles, no
// remote image addresses, and no instruction a person cannot see. A node
// type the client does not know renders its `fallback`.
//
// This file is the source of truth for the document shape and the wire
// types. The Rust validator mirrors it; `WIDGET_LIMITS` are the bounds both
// enforce. The catalog below is the Phase 1 catalog; later types are additive.

export const WIDGET_SCHEMA = "cowboy.widget/1" as const;
export type WidgetSchema = typeof WIDGET_SCHEMA;

/** Fence language of the read-only inline carrier (docs/widgets.md §6.4). */
export const WIDGET_FENCE = "cowboy-widget" as const;

/** Bounds enforced on every document, before and after a patch. Units:
 *  `*Bytes` are UTF-8 bytes of the stored compact JSON; `*Chars` are Unicode
 *  scalar values (not UTF-16 units); `nodeDepth` counts nested nodes and
 *  `jsonDepth` nested JSON values; numbers are finite doubles whose
 *  magnitude is at most 2^53. The 24 KiB document leaves room for the action
 *  envelope inside the 32 KiB re-serialized `cowboy_execute` argument cap
 *  (the raw MCP frame cap is 1 MiB and is not the relevant bound). */
export const WIDGET_LIMITS = {
  documentBytes: 24 * 1024,
  jsonDepth: 32,
  nodes: 400,
  nodeDepth: 12,
  idChars: 64,
  titleChars: 120,
  summaryChars: 200,
  labelChars: 200,
  textChars: 8 * 1024,
  fallbackChars: 2 * 1024,
  /** Inline `data` images: encoded bytes and intrinsic width/height, both
   *  verified by the Controller before the document is stored. */
  inlineImageBytes: 8 * 1024,
  inlineImagePixels: 256,
  tableRows: 200,
  tableColumns: 12,
  /** Aggregate budgets bound rendering work, not just the tree. List items
   *  count recursively. */
  tableCellsPerDocument: 2000,
  itemsPerDocument: 1000,
  imagesPerDocument: 24,
  /** Rows of a table drawn in the transcript preview; the rest open in the inspector. */
  previewRows: 20,
  listItems: 200,
  kvItems: 100,
  steps: 50,
  options: 50,
  /** Values a multiple choice may submit at once. */
  maxSelected: 12,
  cardActions: 6,
  /** Distinct decision groups per document (explicit plus implicit card groups). */
  groups: 32,
  galleryImages: 24,
  patchOps: 64,
  /** Worst-case composed message of any prompt node, computed by
   *  `worstCaseMessageBytes` (the options that contribute the most serialized
   *  bytes, raw and JSON-escaped), measured as UTF-8 bytes; validated at
   *  publish so admission can never refuse a valid document's choice. */
  messageBytes: 2 * 1024,
  /** Serialized `InteractionRecord` with every bounded field at its maximum
   *  (message, selected labels, five attempts, reason, retry links); computed
   *  by `worstCaseRecordBytes` at publish for every prompt node and re-checked
   *  at admission, so later attempts can never push a record past it. A read
   *  page holds at least one record and stays under the 16 KiB result cap. */
  interactionRecordBytes: 10 * 1024,
  reasonChars: 200,
  handleChars: 16,
  stopReasonChars: 64,
  /** A retry may not itself be retried. */
  retryChain: 1,
  /** Dispatch attempts recorded per interaction before it becomes `uncertain`. */
  attemptsPerInteraction: 5,
  /** Target size of one `widgets.read` or lookup page measured on the
   *  complete encoded response, including metadata and escaping; a page
   *  always carries at least one item. */
  readPageBytes: 12 * 1024,
  /** Interaction summaries carried inside a snapshot; full records are paged. */
  snapshotInteractions: 5,
  /** Serialized `WidgetSnapshot`: a maximal document plus every metadata
   *  field at its maximum (`worstCaseSnapshotBytes`), reserved at publish. */
  snapshotBytes: 44 * 1024,
  /** Unsettled interactions a widget may hold at once (one per interactive
   *  node, projected in full on every snapshot); the next admission is
   *  refused `interaction_limit`. */
  outstandingPerWidget: 32,
  /** Revisions, state versions, generations, index versions and sequence
   *  numbers are integers in 0..`maxCounter`; timestamps are epoch
   *  milliseconds in 0..`maxTimestampMs`. */
  maxCounter: 2 ** 31 - 1,
  maxTimestampMs: 2 ** 53 - 1,
  /** The benchmark update class of docs/widgets.md §11: ≤ 3 `set` ops whose
   *  paths and string values are ≤ 64 bytes in JSON-escaped UTF-8 (without
   *  the quotes), a widget id ≤ 64 chars and an operation id ≤ 128 chars,
   *  counters at their maxima; its envelope plus result never exceed this. */
  benchmarkUpdateBytes: 1024,
  interactionsPerWidget: 200,
  widgetsPerSession: 500,
  openWidgetsPerSession: 200,
  lookupIds: 200,
  /** A lookup page; at least `lookupBytes / snapshotBytes` snapshots fit. */
  lookupBytes: 256 * 1024,
  /** Bootstrap entries per page. */
  bootstrapEntries: 500,
} as const;

/** Ids name nodes, groups, option values, agent-chosen widget ids and
 *  client-chosen interaction ids: `[A-Za-z0-9_-]{1,64}`. They are addressing
 *  only: no identifier chosen by an agent is ever copied into a message
 *  attributed to the person. */
export const WIDGET_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;

/** The Controller-assigned public handle of a widget (`w17`), shown in the
 *  row header and used in composed messages instead of the agent's id. */
export const WIDGET_HANDLE_PATTERN = /^w[1-9][0-9]*$/;

/** Controller- and runtime-written metadata strings (`reason`,
 *  `stop_reason`, handles, identifiers) use an alphabet without any
 *  JSON-escaped character, so their serialized size equals their length.
 *  Runtime text (a provider stop reason) is normalized with
 *  `normalizeMetadata` before it is stored. */
export const METADATA_PATTERN = /^[A-Za-z0-9 _.:-]+$/;

/** Maps every character outside `METADATA_PATTERN` to `_` and truncates. */
export function normalizeMetadata(text: string, maxChars: number): string {
  const normalized = Array.from(text)
    .map((character) => (METADATA_PATTERN.test(character) ? character : "_"))
    .join("");
  return normalized.slice(0, maxChars) || "_";
}

/** Visible labels (titles, summaries, interactive labels, option labels)
 *  are one line of printable text: no control characters (`Cc`), no line or
 *  paragraph separators (`Zl`, `Zp`) and no format characters (`Cf`, which
 *  covers bidirectional overrides and zero-width joiners), so the composed
 *  message is exactly two lines, reads as it is drawn and escapes to at most
 *  twice its length. */
export const LABEL_PATTERN = /^[^\p{Cc}\p{Zl}\p{Zp}\p{Cf}]+$/u;

export type Tone = "neutral" | "info" | "success" | "warning" | "error" | "running";
export type Size = "xs" | "sm" | "md" | "lg";

/** Closed glyph set drawn from the product icon font; never an image. */
export type Icon =
  | "info"
  | "check"
  | "warning"
  | "error"
  | "play"
  | "stop"
  | "refresh"
  | "open"
  | "copy"
  | "link"
  | "file"
  | "folder"
  | "image"
  | "table"
  | "list"
  | "clock"
  | "user"
  | "machine"
  | "git"
  | "deploy"
  | "review";

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

/** Where a link points. Images never use a bare remote address (CSP). */
export type Ref =
  /** `https:` only. Opens through the host's remote-link confirmation. */
  | { kind: "url"; url: string }
  /** `seq` is an integer ≥ 0. */
  | { kind: "session"; session_id: string; seq?: number }
  /** Relative to the session's execution root; opens Code Review. `line` is
   *  an integer ≥ 1. */
  | { kind: "file"; path: string; line?: number }
  | { kind: "document"; document_id: string }
  | { kind: "call"; call_id: string }
  /** `/api/artifacts/<64 hex>.<png|jpg|webp|gif|avif>`. */
  | { kind: "artifact"; url: string };

export type ImageSource =
  /** An artifact this owner can already fetch (a tool image, a Call result).
   *  Allowed in published documents, where the Controller verifies and pins
   *  it; not allowed in the inline carrier. */
  | { kind: "artifact"; url: string }
  /** The `index`-th (integer ≥ 0, default 0) image block of the tool result
   *  stored at `seq` (integer ≥ 0) in this session. Every
   *  `publish`/`replace`/`patch` resolves it to an `artifact` source before
   *  storing; the inline carrier resolves it at render time. */
  | { kind: "event"; seq: number; index?: number }
  /** A small still image (icon, glyph): ≤ `inlineImageBytes`, intrinsic size
   *  ≤ `inlineImagePixels` square, no animation; verified by the Controller,
   *  so not allowed in the inline carrier. SVG is deferred. */
  | { kind: "data"; mime: "image/png" | "image/jpeg" | "image/webp"; base64: string };

interface NodeBase {
  /** Required on nodes a patch, a group or a value refers to, and on a card
   *  with `actions` (its implicit group id). */
  id?: string;
  /** Markdown shown when this node type is unknown to the client and in
   *  text-only surfaces (notifications, search, older clients). */
  fallback?: string;
}

export type Node = LayoutNode | ContentNode | StatusNode | MediaNode | InputNode;

// Layout. A container adapts to the width it is actually given (a phone, a
// split Desktop pane, an inspector), never to a surface name; documents
// carry hints, not breakpoints.

export type LayoutNode =
  | (NodeBase & {
      type: "stack";
      direction?: "column" | "row";
      gap?: Size;
      wrap?: boolean;
      /** Keep a row while every child still gets ≥ 120 CSS px; otherwise column. */
      keep_row?: boolean;
      align?: "start" | "center" | "end" | "stretch";
      children: Node[];
    })
  | (NodeBase & {
      type: "card";
      title?: string;
      subtitle?: string;
      status?: { label: string; tone?: Tone };
      /** Footer buttons; one implicit decision group named by the card's
       *  `id` (docs/widgets.md §8.4). */
      actions?: ButtonNode[];
      children: Node[];
    })
  | (NodeBase & { type: "section"; title: string; collapsed?: boolean; children: Node[] })
  | (NodeBase & { type: "divider" });

// Content.

export type ContentNode =
  /** Inline Markdown subset (emphasis, code, links); no blocks. */
  | (NodeBase & {
      type: "text";
      text: string;
      variant?: "title" | "subtitle" | "body" | "caption";
      tone?: Tone;
    })
  | (NodeBase & { type: "kv"; items: KeyValue[] })
  | (NodeBase & { type: "list"; items: ListItem[]; ordered?: boolean; dense?: boolean })
  | (NodeBase & {
      type: "table";
      /** A string is a label with default formatting. */
      columns: (string | Column)[];
      /** One array per row, in column order. */
      rows: Cell[][];
      caption?: string;
      /** Client-side sort by clicking a header. */
      sort?: boolean;
      /** Narrow-width hint. `auto`: fit when the fitted table keeps its minimum
       *  font and target geometry, otherwise one card per row with the first
       *  column as title. `fit` and `cards` are preferences the renderer may
       *  override to keep content readable. */
      mobile?: "auto" | "cards" | "fit";
    })
  | (NodeBase & { type: "link"; label: string; to: Ref; icon?: Icon })
  /** A file chip: opens Code Review at `line` when the session has a worktree. */
  | (NodeBase & { type: "file"; path: string; line?: number; summary?: string });

export type Column = {
  label: string;
  align?: "start" | "center" | "end";
  /** Formats a plain cell value; a `Cell` object may still override tone/link.
   *  Units: `percent` is a fraction 0..1; `bytes` an integer byte count;
   *  `duration` seconds; `date` epoch milliseconds or an RFC 3339 string;
   *  `number` any finite double. A value of the wrong shape renders as text. */
  format?: "text" | "number" | "percent" | "bytes" | "duration" | "date" | "code" | "status";
  /** Prefer shrinking (`min`) or growing (`max`) this column when wide. */
  width?: "auto" | "min" | "max";
};

export type Cell =
  | string
  | number
  | boolean
  | null
  | { v: string | number | boolean | null; tone?: Tone; to?: Ref; title?: string };

export type KeyValue = { k: string; v: string | number | boolean | null; tone?: Tone; to?: Ref };

export type ListItem = {
  text: string;
  tone?: Tone;
  icon?: Icon;
  to?: Ref;
  children?: ListItem[];
};

// Status and progress.

export type StatusNode =
  | (NodeBase & { type: "status"; label: string; tone: Tone; detail?: string })
  | (NodeBase & {
      type: "progress";
      /** Finite, 0..1; omitted means indeterminate. */
      value?: number;
      label?: string;
      detail?: string;
      tone?: Tone;
      variant?: "linear" | "circular";
    })
  | (NodeBase & { type: "steps"; steps: Step[] })
  | (NodeBase & {
      type: "alert";
      severity: "info" | "success" | "warning" | "error";
      title?: string;
      text: string;
    });

export type Step = {
  label: string;
  state: "pending" | "active" | "done" | "failed" | "skipped";
  detail?: string;
};

// Media.

export type MediaNode =
  | (NodeBase & {
      type: "image";
      src: ImageSource;
      /** Required; also the text-only rendering. */
      alt: string;
      caption?: string;
      fit?: "contain" | "cover";
      height?: Size;
    })
  | (NodeBase & {
      type: "gallery";
      images: { src: ImageSource; alt: string; caption?: string }[];
    });

// Input. A tap resolves to an `Action`. The only path back to the agent is
// `prompt`, and it carries no hidden text: the Controller composes the user
// message from the visible label, the selected option labels, the widget's
// public handle and its revision (docs/widgets.md §8.2). Interactive labels
// are unique within a widget and option labels within a choice, so a label
// identifies the node without exposing any agent-chosen identifier.

export type InputNode = ButtonNode | ChoiceNode;

/** Every remotely actionable node names itself: `id` is required on a
 *  button and on a choice so a `widget_action` can address it. */
export type ButtonNode = NodeBase & {
  type: "button";
  id: string;
  /** The visible intent; for `prompt` it is the message the agent receives. */
  label: string;
  action: Action;
  /** `destructive` always confirms. */
  variant?: "primary" | "secondary" | "text" | "destructive";
  icon?: Icon;
  disabled?: boolean;
  /** An agent note shown, labelled as the agent's, inside the product-owned
   *  confirm sheet that previews the exact message to be sent. */
  confirm?: string;
  /** Interactive nodes sharing a group are mutually exclusive: after one
   *  decision is admitted the others are inert until a later revision
   *  changes the group's interaction signature. */
  group?: string;
};

/** One or several options. Selection never sends by itself: the Send control
 *  (Mobile button, Desktop `Mod+Enter`) submits a `prompt` with the selected
 *  option labels, after the exact message has been shown. */
export type ChoiceNode = NodeBase & {
  type: "choice";
  id: string;
  /** The question being answered: required, non-empty, unique among the
   *  widget's interactive labels, and part of the composed message. */
  label: string;
  options: Option[];
  multiple?: boolean;
  /** Integer 1..`maxSelected` whenever present, validated as a raw field;
   *  the effective cardinality is one unless `multiple`
   *  (`effectiveMaxSelected`), and that effective value is what the
   *  interaction signature carries. */
  max_selected?: number;
  /** Initial selection: existing option values, no duplicates, at most the
   *  effective cardinality. */
  value?: string | string[];
  submit?: { label?: string; confirm?: string };
  group?: string;
};

/** `value` follows `WIDGET_ID_PATTERN`, is unique within the choice and is
 *  addressing only; `label` is what the person reads and what the composed
 *  message carries, and is unique within the choice. A choice's initial
 *  `value` must name existing options. */
export type Option = { value: string; label: string; detail?: string; tone?: Tone };

export type Action =
  /** Send what the person saw: the node label (or the selected option
   *  labels) plus the widget handle and revision. */
  | { kind: "prompt" }
  | { kind: "open"; to: Ref }
  /** Copies `text`; the toast shows what was copied. Local, never admitted. */
  | { kind: "copy"; text: string };
// Reserved for a later revision, behind a product confirm and an allowlist:
// { kind: "control"; name: string; input: JsonObject } (docs/widgets.md §12).

export interface WidgetDocument {
  schema: WidgetSchema;
  /** Row and inspector title. */
  title: string;
  /** One line shown while the row is collapsed, in notifications and in the
   *  compatibility projection older clients draw. */
  summary?: string;
  icon?: Icon;
  root: Node;
  presentation?: {
    /** Render inline in the transcript (default) or as a compact row that
     *  opens the inspector. */
    inline?: boolean;
    /** Inline height budget before the fold: sm 24vh, md 40vh (default), lg 60vh. */
    height?: Size;
  };
}

/** The user message the Controller composes for a `prompt` interaction
 *  (docs/widgets.md §8.2). Line one is the visible label; for a choice it
 *  is the question (the choice label) followed by the selected option
 *  labels. Line two names only the public handle, the revision and, for a
 *  choice, the question and the selected labels again. The Rust composer
 *  mirrors this function and the shared fixtures hold both to it. */
export function composeInteractionMessage(
  handle: string,
  revision: number,
  label: string,
  selected?: readonly string[],
): string {
  const reference: { widget: string; revision: number; question?: string; selected?: string[] } = {
    widget: handle,
    revision,
  };
  let first = label;
  if (selected !== undefined) {
    first = `${label} → ${selected.join(", ")}`;
    reference.question = label;
    reference.selected = [...selected];
  }
  return `[widget ${handle} r${revision}] ${first}\n${JSON.stringify(reference)}`;
}

/** Canonical interaction signature of a decision group (docs/widgets.md
 *  §8.4): a decided group resets, and a pending selection is discarded,
 *  exactly when this value changes between revisions. Field order is fixed;
 *  the string is hashed with SHA-256 on both sides. */
export function interactionSignature(nodes: readonly InputNode[]): string {
  const entries = nodes.map((node) =>
    node.type === "button"
      ? { id: node.id, type: node.type, label: node.label, disabled: node.disabled ?? false }
      : {
          id: node.id,
          type: node.type,
          label: node.label,
          multiple: node.multiple ?? false,
          max_selected: effectiveMaxSelected(node),
          options: node.options.map((option) => [option.value, option.label]),
        },
  );
  entries.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return JSON.stringify(entries);
}

/** Effective cardinality of a choice: one unless `multiple`, then
 *  `max_selected` or the limit. */
export function effectiveMaxSelected(node: ChoiceNode): number {
  return node.multiple ? (node.max_selected ?? WIDGET_LIMITS.maxSelected) : 1;
}

const utf8 = (text: string): number => new TextEncoder().encode(text).length;

/** The largest message a prompt node can produce, in UTF-8 bytes. For a
 *  choice, options are ranked by their complete serialized contribution
 *  (the raw label on line one plus its JSON-escaped copy on line two), and
 *  the `effectiveMaxSelected` largest are selected. Both validators use
 *  this rule, so a document that passes publication can never produce a
 *  message admission refuses for size. */
export function worstCaseMessageBytes(node: InputNode): number {
  const handle = "w".padEnd(WIDGET_LIMITS.handleChars, "9");
  const revision = 2 ** 31 - 1;
  if (node.type === "button") return utf8(composeInteractionMessage(handle, revision, node.label));
  const contribution = (label: string): number => utf8(label) + utf8(JSON.stringify(label));
  const selected = [...node.options]
    .map((option) => option.label)
    .sort((a, b) => contribution(b) - contribution(a))
    .slice(0, effectiveMaxSelected(node));
  return utf8(composeInteractionMessage(handle, revision, node.label, selected));
}

// Transport (docs/widgets.md §6). The agent-facing API is discovered with
// cowboy_read({help:"widgets"}); these are its wire types.

export type WidgetChange =
  | { type: "replace"; document: WidgetDocument }
  /** All ops apply or none; the result must satisfy every bound. */
  | { type: "patch"; ops: PatchOp[] }
  /** Terminal for the document and for new interactions; already admitted
   *  interactions still settle (docs/widgets.md §6.6 W5). */
  | { type: "close"; summary?: string };

/** `path` is an RFC 6901 JSON Pointer into the document (`~0`, `~1`
 *  escaping), or `#<node id>` optionally followed by a pointer inside that
 *  node (`#plan/steps/1/state`). `set` replaces an existing value or array
 *  index; `append` adds to an array; `remove` deletes a property or an array
 *  index. Ops apply in order against the running result; a missing path is
 *  `invalid`. */
export type PatchOp =
  | { op: "set"; path: string; value: JsonValue }
  | { op: "remove"; path: string }
  | { op: "append"; path: string; value: JsonValue };

export type PublishArguments = {
  /** Unique within the session (`WIDGET_ID_PATTERN`); generated when omitted. */
  widget_id?: string;
  document: WidgetDocument;
};

export type UpdateArguments = {
  widget_id: string;
  /** The document revision last read; interactions do not advance it. */
  expected_revision: number;
  change: WidgetChange;
};

export type ValidationError = { path: string; code: string; message: string };

export type PublishResult =
  | {
      status: "committed";
      widget_id: string;
      handle: string;
      revision: 1;
      state_version: number;
      warnings: string[];
    }
  | { status: "exists"; widget_id: string; handle: string; revision: number }
  | { status: "invalid"; errors: ValidationError[] }
  /** The session's transcript generation moved while publishing (a Clear
   *  raced the write); nothing was stored. Re-read and publish again. */
  | { status: "cleared" }
  | { status: "refused"; code: "widget_limit" | "rate_limited" | "session_unavailable" };

export type UpdateResult =
  | {
      status: "committed";
      widget_id: string;
      revision: number;
      state_version: number;
      warnings: string[];
    }
  | { status: "revision_conflict"; current: number }
  | { status: "invalid"; errors: ValidationError[] }
  | { status: "unknown_widget" }
  | { status: "closed" }
  | { status: "cleared" }
  | { status: "refused"; code: "rate_limited" | "session_unavailable" };

export type WidgetMeta = {
  widget_id: string;
  handle: string;
  session_id: string;
  title: string;
  summary: string | null;
  /** Document revision: advanced by `publish`/`update` only (CAS key). */
  revision: number;
  /** Advanced by every mutation including interaction settlement and close;
   *  orders snapshots, lookups and client merges. */
  state_version: number;
  /** The session's transcript generation the widget belongs to; Clear
   *  advances it and tombstones every earlier widget. */
  transcript_generation: number;
  closed: boolean;
  /** Anchor seq of the transcript row; null until first published. */
  seq: number | null;
  /** The session's widget index version at creation (docs/widgets.md §6.5). */
  index_version: number;
  published_at_ms: number;
  updated_at_ms: number;
  interactions: number;
  /** Revisions kept readable because a decision was made on them; the list
   *  is read with `select: "revisions"`. */
  pinned_count: number;
};

export type ListArguments = {
  /** Another same-owner session; defaults to the calling session. */
  session_id?: string;
  after?: string;
  limit?: number;
  include_closed?: boolean;
};
export type ListResult = {
  widgets: WidgetMeta[];
  next: string | null;
  transcript_generation: number;
};

/** `widgets.list` pages are packed against the complete encoded response
 *  (`readPageBytes`); `next` continues after the last included entry. */
export type ListPacking = "packed";

/** `widgets.read`. A whole document can exceed the 16 KiB read budget, so
 *  `document` is a text window over the compact JSON (default 1,000 chars,
 *  max 4,000). A current-document read restarts when `revision` changes
 *  between pages; a pinned read (`revision` given) is immutable and its
 *  pages are bound to `document.sha256`, so interaction settlement or later
 *  revisions never interrupt it. Interaction records are paged by cursor
 *  with at least one record per page. */
export type ReadSelection = "meta" | "document" | "interactions" | "revisions";
export type ReadArguments = {
  session_id?: string;
  widget_id: string;
  select: ReadSelection;
  revision?: number;
  offset?: number;
  chars?: number;
  after?: string;
  limit?: number;
};
export type ReadResult =
  | {
      status: "ok";
      meta: WidgetMeta;
      document?: {
        revision: number;
        /** Content identity of the compact JSON being windowed. */
        sha256: string;
        pinned: boolean;
        text: string;
        range: { offset: number; next: number | null; total: number };
      };
      interactions?: { records: InteractionRecord[]; next: string | null };
      revisions?: { pinned: number[]; next: string | null };
    }
  | { status: "unknown_widget" }
  | { status: "cleared" }
  | { status: "unknown_revision" };

/** The person's use of an interactive node, as the agent and the Web see it.
 *  Everything the person saw and chose is recorded immutably with the
 *  revision it was made on; the revision stays readable (`pinned_revisions`). */
export type InteractionRecord = {
  interaction_id: string;
  handle: string;
  node_id: string;
  group: string | null;
  /** Document revision, state version and transcript generation the person
   *  was looking at. */
  revision: number;
  state_version: number;
  transcript_generation: number;
  /** Visible intent: the button label, or the choice label (the question). */
  label: string;
  /** Visible labels of the selected options, for a choice. */
  selected?: string[];
  /** The exact user message the Controller composed. */
  message: string;
  /** SHA-256 of `message`; the dispatcher verifies the queued prompt against it. */
  message_sha256: string;
  at_ms: number;
  /** admitted: recorded durably; the relay keeps it in the session queue.
   *  dispatched: the durable `admitted → dispatched` CAS recorded an
   *  attempt `(command_id, turn_id)` and the prompt was then handed to the
   *  worker; a prompt the runtime itself gives back and requeues (rejected
   *  first ack, `UndeliveredPrompt`) closes the attempt as `returned` and
   *  the record goes back to `admitted`, whereas a prompt the person stops
   *  before any send settles the record `cancelled` (reason `stopped`) and
   *  is never relayed again. delivered: a `TurnEnd` carrying the
   *  attempt's `(turn_id, worker_epoch)` ended with `EndTurn`, `MaxTokens`,
   *  `MaxTurnRequests` or `Refusal` (the transcript echo, `TurnStarted`,
   *  command acks and agent output are never evidence). uncertain: the
   *  attempt ended `Cancelled` or in error, its worker exited without that
   *  `TurnEnd`, the reference is missing, a reset or Clear fenced it, or
   *  the attempt budget is exhausted; never replayed blindly. cancelled:
   *  deleted from the queue (`removed`), fenced by a Clear (`cleared`),
   *  withdrawn by the person's Stop before any send (`stopped`), returned
   *  after a retry was already admitted (`superseded`), or returned after
   *  the interaction's transcript generation was cleared (`cleared`).
   *  failed: refused by the session before the handoff. */
  state: "admitted" | "dispatched" | "delivered" | "uncertain" | "cancelled" | "failed";
  /** ≤ `reasonChars`, `METADATA_PATTERN`. */
  reason?: string;
  /** A durable cancellation intent recorded before the queue entry is
   *  touched; the dispatch CAS requires it to be false. */
  cancel_requested: boolean;
  /** One entry per handoff, written by the dispatch CAS before the prompt
   *  is handed to the worker. `worker_epoch` is bound by the first
   *  `TurnStarted` carrying this `turn_id`. */
  attempts: DispatchAttempt[];
  /** Set on a record created by `widget_action_retry`. */
  retry_of?: string;
  /** Set on an `uncertain`/`failed` record once a retry exists. */
  retried_by?: string;
  /** Set on a retry when a late proof arrives for its original: the original
   *  becomes `delivered` and the retry is a known duplicate (§8.3). */
  duplicate_of?: string;
};

/** Bytes a selected option label adds to the serialized record: once in
 *  `selected` (escaped), once on message line one (escaped inside the
 *  record's message string) and once on message line two (escaped twice,
 *  as JSON inside that string), plus the separators. Additive per label, so
 *  the top-k by this value is the exact worst selection. */
export function recordContribution(label: string): number {
  const once = JSON.stringify(label);
  const twice = JSON.stringify(once);
  return utf8(once) + (utf8(once) - 2) + (utf8(twice) - 2) + 4;
}

function maximalRecord(node: InputNode, selected: string[] | undefined): InteractionRecord {
  const handle = "w".padEnd(WIDGET_LIMITS.handleChars, "9");
  const id = "x".repeat(WIDGET_LIMITS.idChars);
  const attempt: DispatchAttempt = {
    command_id: "c".repeat(WIDGET_LIMITS.idChars),
    turn_id: "t".repeat(WIDGET_LIMITS.idChars),
    worker_epoch: "e".repeat(WIDGET_LIMITS.idChars),
    dispatched_at_ms: WIDGET_LIMITS.maxTimestampMs,
    outcome: "uncertain",
    stop_reason: "s".repeat(WIDGET_LIMITS.stopReasonChars),
    ownership: "released",
    released_by: "fate_rejected_history_unknown",
    delivered_to: "e".repeat(WIDGET_LIMITS.idChars),
  };
  // Every enumerated field is reserved at its longest valid serialization.
  return {
    interaction_id: id,
    handle,
    node_id: node.id,
    group: id,
    revision: WIDGET_LIMITS.maxCounter,
    state_version: WIDGET_LIMITS.maxCounter,
    transcript_generation: WIDGET_LIMITS.maxCounter,
    label: node.label,
    selected,
    message: composeInteractionMessage(handle, WIDGET_LIMITS.maxCounter, node.label, selected),
    message_sha256: "f".repeat(64),
    at_ms: WIDGET_LIMITS.maxTimestampMs,
    state: "dispatched",
    reason: "r".repeat(WIDGET_LIMITS.reasonChars),
    cancel_requested: false,
    attempts: Array.from({ length: WIDGET_LIMITS.attemptsPerInteraction }, () => attempt),
    retry_of: id,
    retried_by: id,
    duplicate_of: id,
  };
}

/** Worst-case serialized size of an interaction record for a prompt node:
 *  the selection that contributes the most serialized bytes
 *  (`recordContribution`), every identifier and counter at its maximum,
 *  five attempts, a maximal ASCII reason and all retry links. Validated at
 *  publish against `interactionRecordBytes`, so no later settlement can
 *  grow a record past the bound. */
export function worstCaseRecordBytes(node: InputNode): number {
  const selected =
    node.type === "choice"
      ? [...node.options]
          .map((option) => option.label)
          .sort((a, b) => recordContribution(b) - recordContribution(a))
          .slice(0, effectiveMaxSelected(node))
      : undefined;
  return utf8(JSON.stringify(maximalRecord(node, selected)));
}

/** Worst-case serialized size of a widget snapshot around `document`: every
 *  identifier and counter at its maximum, 32 decided groups and five
 *  interaction summaries with all retry links. Validated at publish against
 *  `snapshotBytes`; a lookup page always fits `lookupBytes / snapshotBytes`
 *  snapshots. */
export function worstCaseSnapshotBytes(document: WidgetDocument): number {
  const id = "x".repeat(WIDGET_LIMITS.idChars);
  const summary: InteractionSummary = {
    interaction_id: id,
    node_id: id,
    group: id,
    revision: WIDGET_LIMITS.maxCounter,
    state: "dispatched",
    at_ms: WIDGET_LIMITS.maxTimestampMs,
    retry_of: id,
    retried_by: id,
    duplicate_of: id,
  };
  const decided: WidgetSnapshot["decided"] = {};
  for (let index = 0; index < WIDGET_LIMITS.groups; index += 1) {
    decided[`g${String(index).padStart(WIDGET_LIMITS.idChars - 1, "0")}`] = {
      interaction_id: id,
      revision: WIDGET_LIMITS.maxCounter,
    };
  }
  const outstanding: OutstandingInteraction = {
    interaction_id: id,
    node_id: id,
    state: "dispatched",
    ownership: "released",
    retryable: false,
  };
  const snapshot: WidgetSnapshot = {
    widgetId: id,
    handle: "w".padEnd(WIDGET_LIMITS.handleChars, "9"),
    revision: WIDGET_LIMITS.maxCounter,
    stateVersion: WIDGET_LIMITS.maxCounter,
    transcriptGeneration: WIDGET_LIMITS.maxCounter,
    indexVersion: WIDGET_LIMITS.maxCounter,
    seq: WIDGET_LIMITS.maxCounter,
    publishedAtMs: WIDGET_LIMITS.maxTimestampMs,
    updatedAtMs: WIDGET_LIMITS.maxTimestampMs,
    origin: { kind: "mcp", operation: "o".repeat(128) },
    closed: false,
    document,
    documentSha256: "f".repeat(64),
    recentInteractions: Array.from({ length: WIDGET_LIMITS.snapshotInteractions }, () => summary),
    outstanding: Array.from({ length: WIDGET_LIMITS.outstandingPerWidget }, () => outstanding),
    decided,
  };
  return utf8(JSON.stringify(snapshot));
}

/** One handoff of an interaction's prompt to a worker (docs/widgets.md
 *  §8.3 step 5). The runtime already mints `command_id` and `turn_id` for
 *  every dispatch; the Controller records them before the handoff. */
export type DispatchAttempt = {
  command_id: string;
  turn_id: string;
  /** Bound by the first `TurnStarted` for `turn_id`; null until then and
   *  for a Controller-synthesized cancel. */
  worker_epoch: string | null;
  dispatched_at_ms: number;
  /** open: handed off, no attributed end yet; delivered / uncertain: as the
   *  record state; returned: the runtime gave the prompt back before any
   *  provider request (rejected first ack, `UndeliveredPrompt`, a
   *  pre-write capability refusal, or the broker's `rejected` tombstone)
   *  and it is requeued under its cmid unless a retry already exists;
   *  cancelled: withdrawn by the person's Stop before any send, settled
   *  without relay. A widget command is written to exactly one connection
   *  and never retransmitted (§8.3); its epoch is bound by the first
   *  `TurnStarted` or by the broker's `delivered_to`. */
  outcome: "open" | "delivered" | "uncertain" | "returned" | "cancelled";
  /** ≤ `stopReasonChars`, `METADATA_PATTERN` after `normalizeMetadata`. */
  stop_reason?: string;
  /** Transport ownership, separate from delivery (docs/widgets.md §8.3
   *  step 5): `held` from the dispatch CAS until a terminal transport event
   *  releases it; while any attempt of the session is held, no prompt from
   *  any producer may be admitted for the session and an idle snapshot
   *  cannot release the guard. */
  ownership: "held" | "released";
  released_by?: /** The attempt's own `TurnEnd(T, E)`, settled durably before the
   *  runtime acknowledgement. */
    | "turn_end"
    /** `CommandFate.finished`: a terminal receipt from the worker journal. */
    | "fate_finished"
    /** `CommandFate.aborted`: the worker journaled that the prompt never
     *  reached the provider (cancelled before start, configuration failure,
     *  ACP channel failure); the record is `returned`, or `cancelled
     *  (stopped)` when the abort was the person's Stop. */
    | "fate_aborted"
    /** A proven-unsent return before any write, or a Stop before any write. */
    | "returned"
    | "cancelled"
    /** `CommandFate.rejected` after an acknowledged `RevokeCommand`: routing
     *  tombstoned, every recipient epoch fenced or proven terminated, journals
     *  intact and no receipt anywhere. */
    | "fate_rejected"
    /** `CommandFate.rejected` with `journals_unavailable`: future execution
     *  is fenced and every recipient epoch is proven terminated, but the past
     *  cannot be proven; delivery `uncertain`. */
    | "fate_rejected_history_unknown"
    /** `CommandFate.received_exited`: received by an epoch whose process tree
     *  is proven terminated without a terminal receipt; delivery `uncertain`. */
    | "fate_received_exited";
  /** Worker epoch that received the command, from `CommandFate`; null when
   *  no receipt exists. */
  delivered_to?: string | null;
};

/** Machine-side command-fate protocol (docs/widgets.md §8.3 step 5), part of
 *  the `widget_delivery_v1` bundle. Every request and answer carries the
 *  session id and the command id.
 *
 *  Controller → broker: `QueryCommandFate {session_id, command_id}` is a
 *  read: it never installs a tombstone and its answer never releases
 *  ownership unless it is a terminal receipt (`finished`, `aborted`).
 *  `RevokeCommand {session_id, command_id}` installs the routing tombstone,
 *  fences every epoch in the command's durable recipient inventory that is
 *  still connected (`FenceCommand` on each epoch's own FIFO channel, so the
 *  answer is ordered after every earlier write to it), and requires a
 *  confirmed process-tree termination proof for every recipient epoch that
 *  is neither connected nor finished; only then may it answer `rejected`.
 *  Until those proofs exist it answers `pending_epochs`, and ownership stays
 *  held.
 *
 *  Broker ledger per command: `held` (queued, no worker), `routed
 *  {worker_epoch}` recorded under the routing lock before the write to that
 *  epoch's channel (the recipient inventory), `received`/`finished`/`aborted`
 *  relayed from worker journals, `rejected_by_worker` recorded before a
 *  negative ack is forwarded, and the routing tombstone.
 *
 *  Broker → worker: `FenceCommand {session_id, command_id}`; worker → broker:
 *  `CommandFenced {session_id, command_id, state}` where `state` is
 *  `never_received` (a receipt tombstone is installed: a later copy is
 *  refused), `running {turn_id}`, `finished {turn_id, stop_reason}` or
 *  `aborted {reason}`. The worker journal records exactly one disposition
 *  after `received`: `finished` after the `session/prompt` response, or
 *  `aborted {reason}` when the prompt never reached the provider (cancelled
 *  before start, configuration failure, ACP channel failure); it carries the
 *  real command id on every `UndeliveredPrompt`. */
export type CommandFate =
  /** Queued at the broker, no recipient yet; not terminal. */
  | { session_id: string; command_id: string; fate: "held" }
  /** Written to an epoch's channel, no receipt yet; not terminal. */
  | { session_id: string; command_id: string; fate: "routed"; worker_epoch: string }
  /** Received by an epoch whose disposition is not yet known; not terminal. */
  | { session_id: string; command_id: string; fate: "received"; worker_epoch: string }
  /** The current worker is executing it; not terminal. */
  | {
      session_id: string;
      command_id: string;
      fate: "running";
      worker_epoch: string;
      turn_id: string;
    }
  /** Revocation is waiting for fences or termination proofs of these
   *  recipient epochs; not terminal. */
  | { session_id: string; command_id: string; fate: "pending_epochs"; worker_epochs: string[] }
  /** Terminal: the prompt reached the provider and the response arrived. */
  | {
      session_id: string;
      command_id: string;
      fate: "finished";
      worker_epoch: string;
      turn_id: string;
      stop_reason: string;
    }
  /** Terminal: the worker received it but it never reached the provider. */
  | {
      session_id: string;
      command_id: string;
      fate: "aborted";
      worker_epoch: string;
      reason: string;
    }
  /** Terminal, only after an acknowledged `RevokeCommand`: routing
   *  tombstoned, every recipient epoch fenced or proven terminated, no
   *  receipt anywhere. With intact journals the command never executed. */
  | { session_id: string; command_id: string; fate: "rejected"; journals_unavailable: boolean }
  /** Terminal: received by an epoch whose process tree is proven terminated
   *  without a disposition. */
  | { session_id: string; command_id: string; fate: "received_exited"; worker_epoch: string };

/** Control availability for one interaction the renderer must know about
 *  (docs/widgets.md §8.6). Membership: every `admitted` or `dispatched`
 *  record, plus every `uncertain`/`failed` record that is retryable. A
 *  record leaves the list when it settles, when its retry is admitted (the
 *  retry represents it; `retry_of` links back), or when a new decision on a
 *  reset group supersedes it. `retryable` is true only for an
 *  `uncertain`/`failed` record whose attempts have all released ownership,
 *  that has no `retry_of` (chains have length one) and no `retried_by`, and
 *  whose group signature is unchanged. At most one entry per interactive
 *  node by construction. Older records stay readable through
 *  `widgets.read(select:"interactions")`. */
export type OutstandingInteraction = {
  interaction_id: string;
  node_id: string;
  state: InteractionRecord["state"];
  ownership: "held" | "released";
  retryable: boolean;
};

export type InteractionSummary = Pick<
  InteractionRecord,
  | "interaction_id"
  | "node_id"
  | "group"
  | "revision"
  | "state"
  | "at_ms"
  | "retry_of"
  | "retried_by"
  | "duplicate_of"
>;

/** Body of the anchored transcript row. The row itself is carried as a
 *  `notice` update (`title` = widget title, `description` = text projection)
 *  with this body under `_cowboy.widget`, so a client that predates widgets
 *  still draws a readable line while a current client draws the widget. One
 *  row per `widgetId`; every later snapshot replaces it in place. */
export type WidgetSnapshot = {
  widgetId: string;
  handle: string;
  revision: number;
  stateVersion: number;
  transcriptGeneration: number;
  /** The session's widget index version when this widget was created
   *  (docs/widgets.md §6.5): a bootstrap index applies absence only to
   *  widgets created at or before its own index version. */
  indexVersion: number;
  /** Anchor seq once known; null in the first snapshot. */
  seq: number | null;
  publishedAtMs: number;
  updatedAtMs: number;
  origin: { kind: "mcp"; operation: string } | { kind: "repaired" };
  closed: boolean;
  document: WidgetDocument;
  /** Content identity of `document`'s compact JSON. */
  documentSha256: string;
  /** Newest last; bounded by `snapshotInteractions`. Full records: `widgets.read`. */
  recentInteractions: InteractionSummary[];
  /** Every unsettled interaction of the widget (≤ `outstandingPerWidget`),
   *  with its transport ownership, so the renderer can draw the inert and
   *  Send-again states without inferring them from history. Ordered by
   *  `stateVersion`: every ownership change advances it. */
  outstanding: OutstandingInteraction[];
  /** Group id → the interaction that decided it and the revision it was
   *  decided on; at most `groups` entries. */
  decided: Record<string, { interaction_id: string; revision: number }>;
};

/** Whether this session's current worker generation can carry widget
 *  interactions. Carried by the session-level `cowboy_delivery_capabilities`
 *  projection (the same update Steer targets already use), never by a
 *  widget snapshot, so a generation change re-renders every widget's
 *  controls without touching widget state (docs/widgets.md §8.3). Prompt
 *  controls render inert while `widget_delivery` is null. */
export type WidgetDeliveryCapability = {
  widget_delivery: "v1" | null;
  reason?: "worker_capability_missing" | "machine_capability_missing";
};

export type WidgetRow = {
  sessionUpdate: "notice";
  severity: "info";
  title: string;
  description: string;
  _cowboy: { widget: WidgetSnapshot };
};

/** Bootstrap entries for a session's widgets (open and closed, ≤
 *  `widgetsPerSession`). A client discards cached widgets absent from a
 *  complete list and reconciles those whose `state_version` is older. */
export type WidgetStateEntry = {
  widget_id: string;
  handle: string;
  state_version: number;
  closed: boolean;
  seq: number | null;
};
export type WidgetState = {
  /** Controller boot epoch; a new epoch replaces, never merges, cached entries. */
  epoch: string;
  transcript_generation: number;
  /** Monotonic per session; advanced by every widget creation and
   *  tombstone. Pages of one bootstrap share it; absence from the complete
   *  index removes only cached widgets whose `indexVersion` ≤ this value. */
  index_version: number;
  entries: WidgetStateEntry[];
  /** False when more entries follow; `next` fetches them so the client can
   *  always obtain the complete authoritative index. */
  complete: boolean;
  next: string | null;
};

/** `POST /api/sessions/:id/widgets/lookup`. A response from another epoch or
 *  transcript generation than the request is discarded and reconciliation
 *  restarts from bootstrap. At least one `current` snapshot is returned when
 *  any requested id is current; `next` resumes the rest. */
export type WidgetLookupRequest = {
  widget_ids: string[];
  epoch: string;
  transcript_generation: number;
  after?: string;
};
export type WidgetLookupOutcome =
  | { widget_id: string; outcome: "current"; snapshot: WidgetSnapshot }
  /** Tombstoned by a Clear: the client removes its cached row. */
  | { widget_id: string; outcome: "cleared" }
  /** Never existed in this generation: the client removes its cached row. */
  | { widget_id: string; outcome: "unknown" };
export type WidgetLookupResult = {
  epoch: string;
  transcript_generation: number;
  results: WidgetLookupOutcome[];
  next: string | null;
};

/** Client → Controller: a person used a `prompt` node (`open` and `copy`
 *  never reach the Controller). The Controller admits it durably
 *  (idempotent on `interaction_id`), composes the message, relays it to the
 *  session queue and reports through `widget_action_result` and later
 *  snapshots. */
export type WidgetActionCommand = {
  type: "widget_action";
  session_id: string;
  widget_id: string;
  /** What the person saw; a stale tap is refused, never re-targeted. */
  revision: number;
  state_version: number;
  transcript_generation: number;
  node_id: string;
  /** Client-generated; a retry reuses it and receives the original outcome. */
  interaction_id: string;
  /** Selected option values (addressing) per choice node id; the Controller
   *  maps them to labels. A single choice carries one value, a multiple
   *  choice 1..`max_selected`. */
  selected?: Record<string, string[]>;
};

/** Client → Controller: send an `uncertain` (or `failed`) interaction again
 *  under a new id. The retry copies the original's pinned intent (label,
 *  selection, revision, message) regardless of the widget's current
 *  revision, closure or group state, records `retry_of`/`retried_by`, and
 *  is admitted idempotently on `retry_interaction_id` (a replay returns the
 *  retry's current state). Refusals: `cleared`, `not_retryable` (the
 *  original is not `uncertain`/`failed`), `already_retried`, `retry_chain`
 *  (the original is itself a retry), `capability_missing` (the session's
 *  current worker generation cannot carry interactions), `in_flight`,
 *  `interaction_limit`, `rate_limited`, `session_unavailable`, `conflict`.
 *  `in_flight` includes an original whose attempt still holds transport
 *  ownership: a retry is admitted only after ownership is released. A late
 *  proof of the original marks the retry `duplicate_of` it; a broker
 *  `rejected` answer for the original after a retry exists settles the
 *  original `cancelled (superseded)` instead of requeueing it. The
 *  session's ordinary turn Retry routes widget-originated prompts here. */
export type WidgetActionRetryCommand = {
  type: "widget_action_retry";
  session_id: string;
  widget_id: string;
  interaction_id: string;
  retry_interaction_id: string;
};

export type WidgetActionRefusal =
  | "stale_revision"
  | "stale_state"
  | "stale_generation"
  | "closed"
  | "cleared"
  | "unknown_widget"
  | "unknown_node"
  | "not_interactive"
  | "disabled"
  | "group_decided"
  | "in_flight"
  | "invalid_selection"
  | "message_too_large"
  | "interaction_limit"
  | "not_retryable"
  | "already_retried"
  | "retry_chain"
  | "unknown_interaction"
  | "widget_mismatch"
  | "capability_missing"
  | "rate_limited"
  | "session_unavailable"
  | "conflict";

/** Refusals are stateless: nothing is recorded and a retry with the same id
 *  is evaluated afresh. `replayed` is returned only for an admitted id. */
export type WidgetActionResult =
  | {
      type: "widget_action_result";
      interaction_id: string;
      outcome: "admitted";
      state_version: number;
    }
  | {
      type: "widget_action_result";
      interaction_id: string;
      outcome: "replayed";
      state: InteractionRecord["state"];
    }
  | {
      type: "widget_action_result";
      interaction_id: string;
      outcome: "refused";
      refusal: WidgetActionRefusal;
      /** Present for `stale_*`: the snapshot the client should show. */
      current?: WidgetSnapshot;
    };
