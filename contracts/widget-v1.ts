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
// This file is the source of truth for the document shape. The Rust
// validator mirrors it; `WIDGET_LIMITS` are the bounds both enforce. The
// catalog below is the Phase 1 catalog; later types are additive.

export const WIDGET_SCHEMA = "cowboy.widget/1" as const;
export type WidgetSchema = typeof WIDGET_SCHEMA;

/** Fence language of the read-only inline carrier (docs/widgets.md §6.4). */
export const WIDGET_FENCE = "cowboy-widget" as const;

/** Bounds enforced on every document, before and after a patch. Bytes are
 *  UTF-8 of the stored compact JSON; chars are Unicode scalar values; node
 *  depth counts nested nodes, JSON depth counts nested values. The 24 KiB
 *  document leaves room for the action envelope inside the 32 KiB
 *  `cowboy_execute` argument cap. */
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
  inlineImageBytes: 8 * 1024,
  inlineImagePixels: 128,
  tableRows: 200,
  tableColumns: 12,
  /** Aggregate budgets bound rendering work, not just the tree. */
  tableCellsPerDocument: 2000,
  itemsPerDocument: 1000,
  imagesPerDocument: 24,
  /** Rows of a table drawn in the transcript preview; the rest open in the inspector. */
  previewRows: 20,
  listItems: 200,
  kvItems: 100,
  steps: 50,
  options: 50,
  cardActions: 6,
  galleryImages: 24,
  patchOps: 64,
  interactionsRetained: 20,
  widgetsPerSession: 500,
  openWidgetsPerSession: 200,
} as const;

/** Ids name nodes, option values and widgets: `[A-Za-z0-9_-]{1,64}`. An
 *  option value is an identifier, never prose, so it cannot carry an
 *  instruction the person did not see. */
export const WIDGET_ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;

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
  | { kind: "session"; session_id: string; seq?: number }
  /** Relative to the session's execution root; opens Code Review. */
  | { kind: "file"; path: string; line?: number }
  | { kind: "document"; document_id: string }
  | { kind: "call"; call_id: string }
  /** `/api/artifacts/<64 hex>.<png|jpg|webp|gif|avif>`: the exact string form
   *  the artifact garbage collector extracts from stored JSON. */
  | { kind: "artifact"; url: string };

export type ImageSource =
  /** An artifact this owner can already fetch (a tool image, a Call result). */
  | { kind: "artifact"; url: string }
  /** The `index`-th image block of the tool result stored at `seq` in this
   *  session. `widgets.publish` resolves it to an `artifact` source before
   *  storing the document; the inline carrier resolves it at render time. */
  | { kind: "event"; seq: number; index?: number }
  /** A small inline image (icon, glyph): ≤ `inlineImageBytes`, drawn in an
   *  `<img>` box of at most `inlineImagePixels`; SVG stays an opaque image. */
  | {
      kind: "data";
      mime: "image/png" | "image/jpeg" | "image/webp" | "image/svg+xml";
      base64: string;
    };

interface NodeBase {
  /** Required on nodes a patch, an action, a group or a value refers to. */
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
      /** Footer buttons; one implicit decision group (docs/widgets.md §8.4). */
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
  /** Formats a plain cell value; a `Cell` object may still override tone/link. */
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
      /** 0..1; omitted means indeterminate. */
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
// message from what the person saw (docs/widgets.md §8.2).

export type InputNode = ButtonNode | ChoiceNode;

export type ButtonNode = NodeBase & {
  type: "button";
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
   *  changes them. `card.actions` share the card's implicit group. */
  group?: string;
};

/** One or several options. Selection never sends by itself: the Send control
 *  (Mobile button, Desktop `Mod+Enter`) submits a `prompt` with the selected
 *  option values and labels. */
export type ChoiceNode = NodeBase & {
  type: "choice";
  label?: string;
  options: Option[];
  multiple?: boolean;
  value?: string | string[];
  submit?: { label?: string; confirm?: string };
  group?: string;
};

/** `value` follows `WIDGET_ID_PATTERN`. */
export type Option = { value: string; label: string; detail?: string; tone?: Tone };

export type Action =
  /** Send what the person saw: the node label (or selected option labels)
   *  plus the widget id, revision and node id. */
  | { kind: "prompt" }
  | { kind: "open"; to: Ref }
  /** Copies `text`; the toast shows what was copied. */
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

// Transport (docs/widgets.md §6). The agent-facing API is discovered with
// cowboy_read({help:"widgets"}); these are its wire types.

export type WidgetChange =
  | { type: "replace"; document: WidgetDocument }
  /** All ops apply or none; the result must satisfy every bound. */
  | { type: "patch"; ops: PatchOp[] }
  /** Terminal: interactions become inert; no later change is accepted. */
  | { type: "close"; summary?: string };

/** `path` is an RFC 6901 JSON Pointer into the document (`~0`, `~1`
 *  escaping), or `#<node id>` optionally followed by a pointer inside that
 *  node (`#plan/steps/1/state`). `set` replaces an existing value or array
 *  index; `append` adds to an array (`-` is not needed); `remove` deletes a
 *  property or an array index. A missing path is `invalid`. */
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
      revision: 1;
      state_version: number;
      warnings: string[];
    }
  | { status: "exists"; widget_id: string; revision: number }
  | { status: "invalid"; errors: ValidationError[] }
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
  | { status: "refused"; code: "rate_limited" | "session_unavailable" };

export type WidgetMeta = {
  widget_id: string;
  session_id: string;
  title: string;
  summary: string | null;
  /** Document revision: advanced by `publish`/`update` only (CAS key). */
  revision: number;
  /** Advanced by every mutation including interactions and close; orders
   *  snapshots, lookups and client merges. */
  state_version: number;
  closed: boolean;
  seq: number;
  published_at_ms: number;
  updated_at_ms: number;
  interactions: number;
};

/** `widgets.read(widget_id, {select, offset?, chars?})`. A whole document can
 *  exceed the 16 KiB read budget, so the document is read as a text window
 *  of its compact JSON; restart when `state_version` changes between pages. */
export type ReadSelection = "meta" | "document" | "interactions";
export type ReadResult = {
  meta: WidgetMeta;
  document?: { text: string; range: { offset: number; next: number | null; total: number } };
  interactions?: InteractionRecord[];
};

/** The person's use of an interactive node, as the agent and the Web see it.
 *  Everything the person saw and chose is recorded immutably with the
 *  revision it was made on. */
export type InteractionRecord = {
  interaction_id: string;
  node_id: string;
  group: string | null;
  /** Document revision and state version the person was looking at. */
  revision: number;
  state_version: number;
  /** Visible intent: the button label or the choice label. */
  label: string;
  /** Selected option values and their visible labels, for a choice. */
  selection?: {
    values: Record<string, string | string[]>;
    labels: Record<string, string | string[]>;
  };
  /** The exact user message the Controller composed. */
  message: string;
  at_ms: number;
  /** admitted: recorded durably; queued: in the session queue; delivered:
   *  consumed by a turn; cancelled: removed from the queue or fenced by a
   *  transcript clear; failed: refused by the session. */
  state: "admitted" | "queued" | "delivered" | "cancelled" | "failed";
  reason?: string;
};

/** Body of the anchored transcript row. The row itself is carried as a
 *  `notice` update (`title` = widget title, `description` = text projection)
 *  with this body under `_cowboy.widget`, so a client that predates widgets
 *  still draws a readable line while a current client draws the widget. One
 *  row per `widgetId`; every later snapshot replaces it in place. */
export type WidgetSnapshot = {
  widgetId: string;
  revision: number;
  stateVersion: number;
  publishedAtMs: number;
  updatedAtMs: number;
  origin: { kind: "mcp"; operation: string } | { kind: "repaired" };
  closed: boolean;
  document: WidgetDocument;
  /** Newest last; bounded by `interactionsRetained`. */
  interactions: InteractionRecord[];
  /** Group → interaction that decided it. */
  decided: Record<string, string>;
};

export type WidgetRow = {
  sessionUpdate: "notice";
  severity: "info";
  title: string;
  description: string;
  _cowboy: { widget: WidgetSnapshot };
};

/** Bootstrap and `POST /api/sessions/:id/widgets/lookup` entries. */
export type WidgetStateEntry = { widget_id: string; state_version: number; closed: boolean };
export type WidgetState = {
  /** Controller boot epoch; a new epoch replaces, never merges, cached entries. */
  epoch: string;
  entries: WidgetStateEntry[];
  /** False when the session has more widgets than the entry budget. */
  complete: boolean;
};
export type WidgetLookupRequest = { widget_ids: string[] };
export type WidgetLookupResult = {
  entries: WidgetStateEntry[];
  snapshots: WidgetSnapshot[];
  complete: boolean;
};

/** Client → Controller: a person used an interactive node. The Controller
 *  admits it durably (idempotent on `interaction_id`), composes the user
 *  message, relays it to the session queue and reports through
 *  `widget_action_result` and the next snapshot. */
export type WidgetActionCommand = {
  type: "widget_action";
  session_id: string;
  widget_id: string;
  /** What the person saw; a stale tap is refused, never re-targeted. */
  revision: number;
  state_version: number;
  node_id: string;
  /** Client-generated; a retry reuses it and receives the original outcome. */
  interaction_id: string;
  /** Choice selections keyed by choice node id. */
  values?: Record<string, string | string[]>;
};

export type WidgetActionRefusal =
  | "stale_revision"
  | "stale_state"
  | "closed"
  | "cleared"
  | "unknown_widget"
  | "unknown_node"
  | "not_interactive"
  | "group_decided"
  | "in_flight"
  | "invalid_values"
  | "rate_limited"
  | "session_unavailable"
  | "conflict";

export type WidgetActionResult = {
  type: "widget_action_result";
  interaction_id: string;
  outcome: "admitted" | "replayed" | "refused";
  refusal?: WidgetActionRefusal;
  /** Present when refused for staleness: the snapshot the client should show. */
  current?: WidgetSnapshot;
};
