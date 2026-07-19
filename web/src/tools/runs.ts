import type { RenderItem } from "../derive";
import { mcpIdentity } from "./presentation";

export type ToolItem = Extract<RenderItem, { kind: "tool" }>;

export interface ToolRun {
  key: string;
  server: string | null;
  tools: ToolItem[];
}

/** Group only an uninterrupted run of detail-bearing calls to the same MCP
 * server. Thought rows are commentary on the active operation and may sit
 * between calls; every other transcript item is an explicit boundary. */
export function toolRuns(items: RenderItem[]): ToolRun[] {
  const runs: ToolRun[] = [];
  let current: ToolRun | null = null;

  const flush = (): void => {
    if (current) runs.push(current);
    current = null;
  };

  for (const item of items) {
    if (item.kind === "thought" && current?.server) continue;
    if (item.kind !== "tool") {
      flush();
      continue;
    }
    if (item.rawInput === undefined && item.content === undefined) {
      flush();
      continue;
    }
    const mcp = mcpIdentity(item.toolName, item.rawInput, item.title);
    if (mcp && current?.server === mcp.server) {
      current.tools.push(item);
      continue;
    }
    flush();
    current = { key: item.key, server: mcp?.server ?? null, tools: [item] };
  }
  flush();
  return runs;
}

