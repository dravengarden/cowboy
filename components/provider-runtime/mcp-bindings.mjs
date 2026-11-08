// Native bindings are launcher metadata, never execution grants. Probe the
// exact execution-side command once before adding any server to native config.
const namePattern = /^[A-Za-z0-9_-]{1,64}$/;
const absolute = (value) =>
  typeof value === "string" &&
  value.startsWith("/") &&
  value.length <= 4096 &&
  !value.includes("\0");
const object = (value, keys) =>
  value !== null &&
  typeof value === "object" &&
  !Array.isArray(value) &&
  Object.keys(value).length === keys.length &&
  keys.every((key) => Object.hasOwn(value, key));

/**
 * @param {unknown} extensions
 * @param {{managed?: boolean, servers?: Array<string | undefined>, probe?: (argv: string[]) => Promise<{exitCode: number | null, exited: boolean, output: string, output_limit?: boolean}>}} [options]
 */
export async function prepareMcpBindings(
  extensions,
  { managed = false, servers = [], probe } = {},
) {
  if (managed || extensions?.mcp == null) return [];
  const plan = extensions.mcp;
  if (
    !object(plan, ["schema", "command", "bindings"]) ||
    plan.schema !== 1 ||
    !absolute(plan.command) ||
    !Array.isArray(plan.bindings) ||
    plan.bindings.length < 1 ||
    plan.bindings.length > 8
  )
    throw new Error("Invalid native MCP binding plan");
  const names = new Set();
  for (const binding of plan.bindings) {
    if (
      !object(binding, ["name", "descriptor"]) ||
      typeof binding.name !== "string" ||
      !namePattern.test(binding.name) ||
      names.has(binding.name) ||
      !absolute(binding.descriptor)
    )
      throw new Error("Invalid native MCP binding selection");
    names.add(binding.name);
  }
  // Copy before awaiting so a mutable caller cannot change the probed command.
  const command = plan.command;
  const bindings = plan.bindings.map((binding) => ({ ...binding }));
  const occupied = new Set(servers);
  if (typeof probe !== "function") throw new Error("Native MCP capability probe unavailable");
  const result = await probe([command, "mcp", "--binding-capabilities"]);
  if (
    result?.exitCode !== 0 ||
    result.exited !== true ||
    result.output_limit ||
    typeof result.output !== "string" ||
    Buffer.byteLength(result.output) > 4096
  )
    throw new Error("Native MCP capability probe failed");
  let capabilities;
  try {
    capabilities = JSON.parse(result.output);
  } catch {
    throw new Error("Invalid native MCP capability report");
  }
  if (
    !object(capabilities, [
      "schema",
      "descriptor_schema",
      "relay_schema",
      "grant_mcp",
      "transport",
    ]) ||
    capabilities.schema !== 1 ||
    capabilities.descriptor_schema !== 1 ||
    capabilities.relay_schema !== 1 ||
    capabilities.grant_mcp !== true ||
    capabilities.transport !== "stdio_tool_only_v1"
  )
    throw new Error("Incompatible native MCP bridge");
  return bindings.map(({ name, descriptor }) => {
    const base = "cowboy_binding_" + name;
    let nativeName = base;
    for (let suffix = 2; occupied.has(nativeName); suffix++) nativeName = base + "_" + suffix;
    occupied.add(nativeName);
    return {
      name: nativeName,
      scope: "session",
      placement: "target",
      argv: [command, "mcp", "--binding", descriptor],
      env: {},
    };
  });
}
