// Normalize presentation only. The original payload stays on the transcript
// item for Raw/copy; no provider ID or server-specific transport is required.
export function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : undefined;
}

export type ResultBlock =
  | { kind: "text"; text: string; literal?: boolean }
  | { kind: "data"; value: unknown }
  | { kind: "image" | "resource" | "diff"; value: Record<string, unknown> };

export interface ToolResult {
  blocks: ResultBlock[];
  recorded: boolean;
  error: boolean;
}

// Payload identity survives token updates. Closed rows must not repeatedly
// traverse/stringify large MCP results; the caches release evicted history.
const resultCache = new WeakMap<object, ToolResult>();
const errorCache = new WeakMap<object, boolean>();

function parsedData(text: string): unknown {
  if (!text.trimStart().startsWith("{") && !text.trimStart().startsWith("[")) {
    return undefined;
  }
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

export function toolResult(content: unknown): ToolResult {
  if (content !== null && typeof content === "object") {
    const cached = resultCache.get(content);
    if (cached) return cached;
  }
  const result: ToolResult = {
    blocks: [],
    recorded: content !== undefined,
    error: false,
  };
  const data = (value: unknown): void => {
    result.blocks.push({ kind: "data", value });
  };
  const visit = (value: unknown, raw = false, depth = 0): void => {
    if (value === undefined) return;
    if (depth > 12) {
      data(value);
      return;
    }
    if (typeof value === "string") {
      if (value) result.blocks.push({ kind: "text", text: value });
      return;
    }
    if (Array.isArray(value)) {
      if (
        raw &&
        (!value.length ||
          !value.every((block) => typeof record(block)?.type === "string"))
      ) {
        data(value);
      } else {
        value.forEach((block) => visit(block, false, depth + 1));
      }
      return;
    }
    const object = record(value);
    if (!object) {
      data(value);
      return;
    }
    if (object.type === "tool_result") {
      visit(object.value, true, depth + 1);
      return;
    }
    if (object.type === "content") {
      visit(object.content, false, depth + 1);
      return;
    }
    // Codex ACP's opaque MCP result/error envelope.
    if ("result" in object && "error" in object && !object.type) {
      if (object.error != null) {
        result.error = true;
        data(object.error);
      }
      if (object.result != null) visit(object.result, true, depth + 1);
      return;
    }
    // Native MCP CallToolResult, with or without ACP's content wrapper.
    if (
      !object.type &&
      (Array.isArray(object.content) || "structuredContent" in object)
    ) {
      result.error ||= object.isError === true;
      const structured = object.structuredContent;
      const hasStructured = structured !== undefined && structured !== null;
      const blocks = Array.isArray(object.content) ? object.content : [];
      for (const block of blocks) {
        const text = record(block)?.text;
        // Matrix and other MCPs send the same JSON as both text and structured
        // content. Paint it once, retaining any additional prose/media blocks.
        if (
          hasStructured && typeof text === "string" &&
          JSON.stringify(parsedData(text)) === JSON.stringify(structured)
        ) continue;
        visit(block, false, depth + 1);
      }
      if (hasStructured) data(structured);
      return;
    }
    if (
      (object.type === "text" || object.type === "raw_output" ||
        !object.type) && typeof object.text === "string"
    ) {
      if (object.text) {
        result.blocks.push({
          kind: "text",
          text: object.text,
          literal: object.type === "raw_output",
        });
      }
    } else if (object.type === "image") {
      result.blocks.push({ kind: "image", value: object });
    } else if (object.type === "diff") {
      result.blocks.push({ kind: "diff", value: object });
    } else if (
      object.type === "resource" || object.type === "resource_link" ||
      object.type === "audio"
    ) {
      result.blocks.push({ kind: "resource", value: object });
    } else {
      data(value);
    }
  };
  visit(content);
  if (content !== null && typeof content === "object") {
    resultCache.set(content, result);
  }
  return result;
}

export function resultText(content: unknown): string {
  return toolResult(content).blocks.map((block) => {
    if (block.kind === "text") return block.text;
    if (block.kind === "diff") {
      const before = block.value.oldText;
      const after = block.value.newText;
      return [before && `--- before\n${before}`, after && `+++ after\n${after}`]
        .filter(Boolean).join("\n");
    }
    if (block.kind === "resource") {
      const resource = record(block.value.resource) ?? block.value;
      return [resource.name, resource.uri, resource.text].filter((value) =>
        typeof value === "string"
      ).join("\n");
    }
    return block.kind === "data" ? JSON.stringify(block.value, null, 2) : "";
  }).filter(Boolean).join("\n");
}

export function jsonTextValue(text: string): unknown {
  return parsedData(text);
}

/** Mirror server projection for legacy/uncompacted events in client storage. */
export function toolContent(update: Record<string, unknown>): unknown {
  const content = update.content;
  if (content != null && (!Array.isArray(content) || content.length > 0)) {
    return content;
  }
  if ("rawOutput" in update) {
    const raw = update.rawOutput;
    const formatted = record(raw)?.formatted_output;
    return typeof formatted === "string"
      ? [{ type: "raw_output", text: formatted }]
      : [{ type: "tool_result", value: raw }];
  }
  return content;
}

export function resultStatus(status: string, content: unknown): string {
  return status === "completed" && hasToolError(content) ? "failed" : status;
}

function hasToolError(content: unknown, depth = 0): boolean {
  if (content === null || typeof content !== "object" || depth > 12) {
    return false;
  }
  const cached = errorCache.get(content);
  if (cached !== undefined) return cached;
  const value = record(content);
  const error = Array.isArray(content)
    ? content.some((block) => hasToolError(block, depth + 1))
    : value?.type === "tool_result"
    ? hasToolError(value.value, depth + 1)
    : value?.type === "content"
    ? hasToolError(value.content, depth + 1)
    : value && !value.type && "result" in value && "error" in value
    ? value.error != null || hasToolError(value.result, depth + 1)
    : !!(value && !value.type &&
      (Array.isArray(value.content) || "structuredContent" in value) &&
      (value.isError === true || hasToolError(value.content, depth + 1)));
  errorCache.set(content, error);
  return error;
}
