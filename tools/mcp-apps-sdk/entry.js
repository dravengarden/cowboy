// The view's SDK surface: bundled into one inline IIFE by
// tools/mcp-apps-conformance.ts, as a signed single-file view would ship it.
import { App, PostMessageTransport } from "@modelcontextprotocol/ext-apps";

globalThis.McpAppsSdk = { App, PostMessageTransport };
