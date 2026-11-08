/** Closed Plugin vocabulary shared by inventory and compatibility readers. */
export const MACHINE_PLUGIN_KINDS = [
  "agent_provider",
  "code_intelligence",
  "telemetry_backend",
  "workspace_extension",
  "memory_backend",
  "call_profile",
  "mcp_server",
  "machine_browser",
] as const;
export const PLUGIN_KINDS = [...MACHINE_PLUGIN_KINDS, "authentication_provider"] as const;
export type MachinePluginKind = (typeof MACHINE_PLUGIN_KINDS)[number];
export type PluginKind = (typeof PLUGIN_KINDS)[number];
export const machinePluginKinds = new Set<string>(MACHINE_PLUGIN_KINDS);
export const pluginKinds = new Set<string>(PLUGIN_KINDS);
