import { providerName } from "../providerPresentation";
import { record, resultText } from "./result";

export { providerName } from "../providerPresentation";

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
  const input = record(rawInput) ?? {};
  const explicitServer = typeof input.server === "string" ? input.server : "";
  const explicitTool = typeof input.tool === "string" ? input.tool : "";
  const encoded = /^(?:functions\.)?mcp__(.+?)__(.+)$/u.exec(toolName);
  const dotted = /^mcp\.([^.]+)\.(.+)$/u.exec(toolName) ?? /^mcp\.([^.]+)\.(.+)$/u.exec(title);
  const server = encoded?.[1] || dotted?.[1] || explicitServer;
  const tool = encoded?.[2] || dotted?.[2] || explicitTool;
  if (!server || !tool) return null;
  // A qualified tool's own fields may be named `server`, `tool` or
  // `arguments`. Only unwrap the explicit ACP transport envelope.
  const envelope = explicitServer === server && explicitTool === tool;
  let args = envelope ? record(input.arguments) ?? {} : input;
  if (envelope && typeof input.arguments === "string") {
    try { args = record(JSON.parse(input.arguments)) ?? { arguments: input.arguments }; }
    catch { args = { arguments: input.arguments }; }
  }
  return { server, tool, arguments: args };
}

export function mcpHeading(mcp: McpIdentity): string {
  const args = mcp.arguments;
  if (mcp.server === "cowboy") {
    if (mcp.tool === "cowboy_read" && typeof args.help === "string") return `Cowboy help · ${args.help}`;
    if (mcp.tool === "cowboy_execute" && typeof args.action === "string") return `Cowboy · ${args.action}`;
  }
  if (mcp.server === "matrix") {
    if (mcp.tool === "memory_read" && typeof args.help === "string") return `Matrix help · ${args.help}`;
    const titles: Record<string, string> = {
      memory_search: "Search memory", memory_get: "Read memory", memory_put: "Save memory",
      memory_forget: "Forget memory", memory_read: "Read memory", memory_execute: "Update memory",
      memory_receipt: "Memory receipt",
    };
    if (titles[mcp.tool]) return titles[mcp.tool]!;
  }
  return humanize(mcp.tool);
}

function humanize(value: string): string {
  const words = value.replace(/[_-]+/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : "Tool";
}

export function toolTypeName(
  _provider: string,
  toolName: string,
  kind: string,
): string {
  if (toolName) return toolName;
  if (kind === "execute") return "Shell";
  if (kind === "edit") return "Patch";
  if (kind === "read") return "Read";
  return kind ? kind.charAt(0).toUpperCase() + kind.slice(1) : "Tool";
}

export function toolUsesRawOnly({ kind, toolName = "", title = "", rawInput }: {
  kind: string;
  toolName?: string;
  title?: string;
  rawInput?: unknown;
}): boolean {
  if (mcpIdentity(toolName, rawInput, title)) return false;
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
  if (mcp) return mcpHeading(mcp);
  if (kind === "search") return "Web search";
  if (kind === "execute") {
    if (typeof input.description === "string" && input.description.trim()) {
      return input.description.trim();
    }
    return `${toolTypeName(provider, toolName, kind)} command`;
  }
  return title || toolTypeName(provider, toolName, kind);
}

export function toolVariantLabel(
  { provider, providerVersion, providerDigest, toolName, kind, title, rawInput }: {
    provider: string;
    providerVersion?: string | undefined;
    providerDigest?: string | undefined;
    toolName: string;
    kind: string;
    title: string;
    rawInput?: unknown;
  },
): string {
  const mcp = mcpIdentity(toolName, rawInput, title);
  const name = providerName(provider, providerVersion, providerDigest);
  if (mcp) return `${name} · MCP · ${mcp.server}`;
  return `${name} · ${
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
    [mcp.arguments.code, mcp.arguments.function, mcp.arguments.query, mcp.arguments.url,
      mcp.arguments.action, mcp.arguments.help, mcp.arguments.title, mcp.arguments.id, mcp.arguments.operation]
      .find((value) => typeof value === "string");
  const primary = typeof command === "string"
    ? command
    : Array.isArray(command)
    ? command.map(String).join(" ")
    : typeof mcpPrimary === "string"
    ? mcpPrimary
    : mcp ? mcpHeading(mcp) : title;
  return [primary, resultText(content)].filter(Boolean).join("\n\n");
}
