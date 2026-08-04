import { providerName } from "../providerPresentation";

export { providerName } from "../providerPresentation";

function contentText(content: unknown): string {
  const blocks = Array.isArray(content)
    ? content
    : content == null
    ? []
    : [content];
  return blocks.map((block) => {
    if (!block || typeof block !== "object") {
      return typeof block === "string" ? block : "";
    }
    const value = block as Record<string, unknown>;
    if (typeof value.text === "string") return value.text;
    const nested = value.content;
    if (
      nested && typeof nested === "object" &&
      typeof (nested as Record<string, unknown>).text === "string"
    ) {
      return String((nested as Record<string, unknown>).text);
    }
    if (value.type === "diff") {
      const before = typeof value.oldText === "string" ? value.oldText : "";
      const after = typeof value.newText === "string" ? value.newText : "";
      return [before && `--- before\n${before}`, after && `+++ after\n${after}`]
        .filter(Boolean).join("\n");
    }
    return "";
  }).filter(Boolean).join("\n");
}

export interface McpIdentity {
  server: string;
  tool: string;
  arguments: Record<string, unknown>;
}

export function mcpIdentity(
  toolName: string,
  rawInput?: unknown,
  title = "",
): McpIdentity | null {
  const input = rawInput && typeof rawInput === "object"
    ? rawInput as Record<string, unknown>
    : {};
  const explicitServer = typeof input.server === "string" ? input.server : "";
  const explicitTool = typeof input.tool === "string" ? input.tool : "";
  const encoded = toolName.startsWith("mcp__") ? toolName.split("__") : [];
  const dotted = title.startsWith("mcp.") ? title.split(".") : [];
  const server = explicitServer || encoded[1] || dotted[1] || "";
  const tool = explicitTool || encoded[2] || dotted[2] || "";
  if (!server || !tool) return null;
  const args = input.arguments && typeof input.arguments === "object" &&
      !Array.isArray(input.arguments)
    ? input.arguments as Record<string, unknown>
    : explicitServer || explicitTool
    ? {}
    : input;
  return { server, tool, arguments: args };
}

function humanize(value: string): string {
  const words = value.replace(/[_-]+/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : "Tool";
}

export function toolTypeName(
  provider: string,
  toolName: string,
  kind: string,
): string {
  if (toolName) return toolName;
  if (provider === "codex" || provider === "codex-deepseek") {
    if (kind === "execute") return "Shell";
    if (kind === "edit") return "Patch";
    if (kind === "read") return "Read";
  }
  return kind ? kind.charAt(0).toUpperCase() + kind.slice(1) : "Tool";
}

export function toolUsesRawOnly({ kind, toolName = "", title = "" }: {
  kind: string;
  toolName?: string;
  title?: string;
}): boolean {
  const compact = (value: string): string =>
    value.toLowerCase().replace(/[^a-z0-9]+/g, "");
  const normalizedKind = compact(kind);
  const normalizedTool = compact(toolName);

  return normalizedKind === "search" ||
    normalizedTool === "websearch" ||
    normalizedTool === "searchquery" ||
    /^web\s+search(?:\s*:|$)/iu.test(title.trim());
}

export function toolHeading({ provider, toolName, kind, title, rawInput }: {
  provider: string;
  toolName: string;
  kind: string;
  title: string;
  rawInput?: unknown;
}): string {
  const input = rawInput && typeof rawInput === "object"
    ? rawInput as Record<string, unknown>
    : {};
  const mcp = mcpIdentity(toolName, rawInput, title);
  if (mcp) return humanize(mcp.tool);
  if (kind === "search") return "Web search";
  if (kind === "execute") {
    if (
      (provider === "claude-code" || provider === "claude-deepseek") &&
      typeof input.description === "string" &&
      input.description.trim()
    ) {
      return input.description.trim();
    }
    return `${toolTypeName(provider, toolName, kind)} command`;
  }
  return title || toolTypeName(provider, toolName, kind);
}

export function toolVariantLabel(
  { provider, toolName, kind, title, rawInput }: {
    provider: string;
    toolName: string;
    kind: string;
    title: string;
    rawInput?: unknown;
  },
): string {
  const mcp = mcpIdentity(toolName, rawInput, title);
  if (mcp) return `${providerName(provider)} · MCP · ${mcp.server}`;
  return `${providerName(provider)} · ${
    toolTypeName(provider, toolName, kind)
  }`;
}

export function toolCopyText({ title, toolName = "", rawInput, content }: {
  title: string;
  toolName?: string;
  rawInput?: unknown;
  content?: unknown;
}): string {
  const input = rawInput && typeof rawInput === "object"
    ? rawInput as Record<string, unknown>
    : {};
  const command = input.command ?? input.cmd;
  const mcp = mcpIdentity(toolName, rawInput, title);
  const mcpPrimary = mcp &&
    [mcp.arguments.function, mcp.arguments.query, mcp.arguments.url]
      .find((value) => typeof value === "string");
  const primary = typeof command === "string"
    ? command
    : Array.isArray(command)
    ? command.map(String).join(" ")
    : typeof mcpPrimary === "string"
    ? mcpPrimary
    : title;
  return [primary, contentText(content)].filter(Boolean).join("\n\n");
}
