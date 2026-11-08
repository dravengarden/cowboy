// Cowboy Widgets v1: the agent-authored structured UI document.
//
// Design: docs/widgets.md. An agent emits this JSON through Cowboy MCP
// (`widgets.publish` / `widgets.update`) or inside a ```cowboy-widget fence.
// Cowboy validates it at both trust boundaries (Controller in Rust, Web in
// TypeScript, shared fixtures) and renders it with product MUI components.
// Nothing in a document is executed: no markup, no scripts, no styles, no
// remote image addresses. Unknown node types render their `fallback`.
//
// This file is the source of truth for the document shape. The Rust
// validator mirrors it; `WIDGET_LIMITS` are the bounds both enforce.

export const WIDGET_SCHEMA = "cowboy.widget/1" as const;
export type WidgetSchema = typeof WIDGET_SCHEMA;

/** Fence language of the read-only inline carrier (docs/widgets.md §5.4). */
export const WIDGET_FENCE = "cowboy-widget" as const;

/** `sessionUpdate` of the anchored transcript row (docs/widgets.md §5.2). */
export const WIDGET_UPDATE = "cowboy_widget" as const;

/** Bounds enforced on every document, before and after a patch. Serialized
 *  size is UTF-8 bytes of the canonical JSON; the 24 KiB document leaves room
 *  for the action envelope inside the 32 KiB `cowboy_execute` argument cap. */
export const WIDGET_LIMITS = {
  documentBytes: 24 * 1024,
  nodes: 400,
  depth: 12,
  idChars: 64,
  titleChars: 120,
  summaryChars: 200,
  labelChars: 200,
  textChars: 8 * 1024,
  markdownChars: 16 * 1024,
  fallbackChars: 2 * 1024,
  inlineImageBytes: 8 * 1024,
  tableRows: 200,
  tableColumns: 12,
  listItems: 200,
  kvItems: 100,
  steps: 50,
  options: 50,
  formFields: 24,
  cardActions: 6,
  galleryImages: 24,
  distributionSegments: 24,
  patchOps: 64,
  promptChars: 2 * 1024,
  valuesBytes: 4 * 1024,
  interactionsRetained: 20,
  openWidgetsPerSession: 200,
} as const;

/** Ids name nodes for patches, actions and form values: `[A-Za-z0-9_-]{1,64}`. */
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
  | "chart"
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
  /** `<64 hex>.<png|jpg|webp|gif|avif>` served by `/api/artifacts/`. */
  | { kind: "artifact"; name: string };

export type ImageSource =
  /** An artifact this owner can already fetch (a Call result, a tool image). */
  | { kind: "artifact"; name: string }
  /** The `index`-th image block of the tool result stored at `seq` in this
   *  session. Resolved by the Web from the stored event; no bytes travel. */
  | { kind: "event"; seq: number; index?: number }
  /** A small inline image (icon, glyph). Bounded by `inlineImageBytes`. */
  | {
      kind: "data";
      mime: "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/svg+xml";
      base64: string;
    };

interface NodeBase {
  /** Required on nodes a patch, an action or a field refers to. */
  id?: string;
  /** Markdown shown when this node type or a required property is unknown
   *  to the client, and in text-only surfaces (notifications, search). */
  fallback?: string;
}

export type Node = LayoutNode | ContentNode | StatusNode | MediaNode | InputNode;

// Layout. Each container decides its own narrow-width behaviour; documents
// never describe breakpoints. A row stack becomes a column below 480 CSS px
// unless `keep_row` is set.

export type LayoutNode =
  | (NodeBase & {
      type: "stack";
      direction?: "column" | "row";
      gap?: Size;
      wrap?: boolean;
      keep_row?: boolean;
      align?: "start" | "center" | "end" | "stretch";
      children: Node[];
    })
  | (NodeBase & {
      type: "grid";
      /** Columns per surface width; defaults `{xs:1, sm:2, md:3}`. */
      columns?: { xs?: number; sm?: number; md?: number };
      gap?: Size;
      children: Node[];
    })
  | (NodeBase & {
      type: "card";
      title?: string;
      subtitle?: string;
      status?: { label: string; tone?: Tone };
      /** Footer buttons; bounded by `cardActions`. */
      actions?: ButtonNode[];
      children: Node[];
    })
  | (NodeBase & { type: "section"; title: string; collapsed?: boolean; children: Node[] })
  | (NodeBase & {
      type: "tabs";
      selected?: string;
      tabs: { id: string; label: string; children: Node[] }[];
    })
  | (NodeBase & { type: "divider" });

// Content.

/** Inline text accepts the Markdown inline subset (emphasis, code, links). */
export type ContentNode =
  | (NodeBase & {
      type: "text";
      text: string;
      variant?: "title" | "subtitle" | "body" | "caption";
      tone?: Tone;
    })
  /** Full Markdown through the ordinary renderer (tables, fences, math). */
  | (NodeBase & { type: "markdown"; text: string })
  | (NodeBase & { type: "code"; text: string; lang?: string; title?: string; wrap?: boolean })
  | (NodeBase & { type: "kv"; items: KeyValue[]; columns?: 1 | 2 })
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
      /** Narrow-width presentation. `auto`: fit when ≤ 3 columns and cells
       *  are short, otherwise one card per row with the first column as title. */
      mobile?: "auto" | "cards" | "fit";
    })
  /** Collapsible, selectable JSON (the `StructuredResult` presentation). */
  | (NodeBase & { type: "json"; value: JsonValue; title?: string })
  | (NodeBase & { type: "diff"; unified: string; path?: string })
  | (NodeBase & {
      type: "metric";
      label: string;
      value: string | number;
      unit?: string;
      delta?: { value: string | number; direction?: "up" | "down" | "flat"; tone?: Tone };
      hint?: string;
    })
  | (NodeBase & { type: "link"; label: string; to: Ref; icon?: Icon })
  /** A file chip: opens Code Review at `line` when the session has a worktree. */
  | (NodeBase & { type: "file"; path: string; line?: number; summary?: string });

export type Column = {
  label: string;
  align?: "start" | "center" | "end";
  /** Formats a plain cell value; a `Cell` object may still override tone/link. */
  format?: "text" | "number" | "percent" | "bytes" | "duration" | "date" | "code" | "status";
  /** Prefer shrinking (`min`) or growing (`max`) this column on wide screens. */
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
    })
  /** A stacked bar of parts of a whole (disk, quota, test outcomes). */
  | (NodeBase & { type: "distribution"; segments: Segment[]; total?: number; unit?: string });

export type Step = {
  label: string;
  state: "pending" | "active" | "done" | "failed" | "skipped";
  detail?: string;
};

export type Segment = { label: string; value: number; tone?: Tone };

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
    })
  /** Mermaid source, rendered with the existing strict diagram renderer and
   *  its source fallback. */
  | (NodeBase & { type: "diagram"; source: string });

// Input. Every interactive node resolves to an `Action`; the Web never does
// anything a person could not do by hand in the same session.

export type InputNode = ButtonNode | ChoiceNode | FormNode;

export type ButtonNode = NodeBase & {
  type: "button";
  label: string;
  action: Action;
  variant?: "primary" | "secondary" | "text" | "destructive";
  icon?: Icon;
  disabled?: boolean;
  /** Shown in a confirm sheet before the action runs. */
  confirm?: Confirm;
};

/** One or several options; a single choice submits on selection when
 *  `submit` is set, otherwise the selection waits for a sibling button
 *  whose action reads the widget's current values. */
export type ChoiceNode = NodeBase & {
  type: "choice";
  label?: string;
  options: Option[];
  multiple?: boolean;
  value?: string | string[];
  submit?: Action;
};

export type FormNode = NodeBase & {
  type: "form";
  title?: string;
  fields: Field[];
  submit: { label: string; action: Action; confirm?: Confirm };
};

export type Option = { value: string; label: string; detail?: string; tone?: Tone };

export type Confirm = { title: string; text?: string; confirm_label?: string };

export type Field =
  | {
      id: string;
      kind: "text";
      label: string;
      placeholder?: string;
      multiline?: boolean;
      required?: boolean;
      value?: string;
      max_chars?: number;
    }
  | {
      id: string;
      kind: "number";
      label: string;
      min?: number;
      max?: number;
      step?: number;
      required?: boolean;
      value?: number;
    }
  | {
      id: string;
      kind: "select";
      label: string;
      options: Option[];
      multiple?: boolean;
      required?: boolean;
      value?: string | string[];
    }
  | { id: string; kind: "toggle"; label: string; value?: boolean };

/** What a tap does. `prompt` is the only way back to the agent: the
 *  Controller composes a user message from `text`, the interacting node and
 *  the widget's current values (docs/widgets.md §7.2). */
export type Action =
  | { kind: "prompt"; text: string; data?: JsonObject }
  | { kind: "open"; to: Ref }
  | { kind: "copy"; text: string };
// Reserved for a later revision, behind a confirm sheet and an allowlist:
// { kind: "control"; name: string; input: JsonObject } (docs/widgets.md §11).

export interface WidgetDocument {
  schema: WidgetSchema;
  /** Row and inspector title. */
  title: string;
  /** One line shown while the row is collapsed and in notifications. */
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

// Transport (docs/widgets.md §5). The agent-facing API is discovered with
// cowboy_read({help:"widgets"}); these are its wire types.

export type WidgetChange =
  | { type: "replace"; document: WidgetDocument }
  | { type: "patch"; ops: PatchOp[] }
  /** Interactions become inert; the row keeps rendering. */
  | { type: "close"; summary?: string }
  | { type: "reopen" };

/** `path` is a JSON Pointer into the document, or `#<node id>` optionally
 *  followed by a pointer inside that node (`#progress/value`). */
export type PatchOp =
  | { op: "set"; path: string; value: JsonValue }
  | { op: "remove"; path: string }
  | { op: "append"; path: string; value: JsonValue };

export type PublishArguments = {
  /** Stable id chosen by the agent (`WIDGET_ID_PATTERN`); generated when omitted. */
  widget_id?: string;
  document: WidgetDocument;
};

export type UpdateArguments = {
  widget_id: string;
  /** The document revision last read; interactions do not advance it. */
  expected_revision: number;
  change: WidgetChange;
};

export type PublishResult =
  | { status: "committed"; widget_id: string; revision: number; seq: number; warnings: string[] }
  | { status: "invalid"; errors: ValidationError[] };

export type UpdateResult =
  | { status: "committed"; widget_id: string; revision: number; warnings: string[] }
  | { status: "revision_conflict"; current: number }
  | { status: "invalid"; errors: ValidationError[] }
  | { status: "unknown_widget" };

export type ValidationError = { path: string; code: string; message: string };

export type WidgetMeta = {
  widget_id: string;
  session_id: string;
  title: string;
  summary: string | null;
  revision: number;
  closed: boolean;
  seq: number;
  published_at_ms: number;
  updated_at_ms: number;
  interactions: number;
};

export type WidgetView = WidgetMeta & {
  document: WidgetDocument;
  /** Newest last; bounded by `interactionsRetained`. */
  recent_interactions: Interaction[];
};

/** A person's use of an interactive node, as the agent and the Web see it. */
export type Interaction = {
  interaction_id: string;
  node_id: string;
  at_ms: number;
  /** `queued`: a user message is waiting for the agent; `delivered`: the
   *  agent's turn consumed it; `failed`: refused or expired. */
  state: "queued" | "delivered" | "failed";
  values?: JsonObject;
};

/** The anchored transcript row. One row per `widgetId`; every later snapshot
 *  replaces it in place at the first row's seq. */
export type WidgetSnapshot = {
  sessionUpdate: typeof WIDGET_UPDATE;
  widgetId: string;
  revision: number;
  /** Controller-stamped ordering key across snapshots and reconciliation. */
  observedSeq: number;
  publishedAtMs: number;
  updatedAtMs: number;
  origin: { kind: "mcp"; operation: string } | { kind: "repaired" };
  closed: boolean;
  document: WidgetDocument;
  interactions: Interaction[];
};

/** Client → Controller: a person used an interactive node. The Controller
 *  validates against the current snapshot, composes the user message, queues
 *  it and reports the interaction state through the next snapshot. */
export type WidgetActionCommand = {
  type: "widget_action";
  session_id: string;
  widget_id: string;
  /** The revision the person saw; a stale click is refused, never re-targeted. */
  revision: number;
  node_id: string;
  /** Client-generated; a retry reuses it. */
  interaction_id: string;
  values?: JsonObject;
};
