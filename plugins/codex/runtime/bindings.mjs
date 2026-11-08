import { prepareMcpBindings } from "../../../components/provider-runtime/mcp-bindings.mjs";
import {
  probeExecutionCli,
  verifyExecutionCli,
} from "../../../components/provider-runtime/execution-probe.mjs";

export { verifyExecutionCli };

export async function codexMcpBindings(extensions, descriptor, servers, probe = probeExecutionCli) {
  if (extensions?.mcp == null) return [];
  if (!descriptor || descriptor.binding.managed) {
    throw new Error("Native MCP bindings require an ordinary bound execution");
  }
  return prepareMcpBindings(extensions, {
    servers,
    probe: (argv) => probe(descriptor, argv),
  });
}

export function bindingConfiguration(entries, descriptor) {
  if (!entries.length) return [];
  const environment = descriptor?.binding?.environment?.id;
  const cwd = descriptor?.binding?.workspace?.cwd;
  if (!environment || typeof cwd !== "string" || !cwd.startsWith("/")) {
    throw new Error("Native MCP bindings require target placement");
  }
  return entries.flatMap(({ name, argv }) => [
    "-c",
    `mcp_servers.${name}={command=${JSON.stringify(argv[0])},args=${JSON.stringify(argv.slice(1))},environment_id=${JSON.stringify(environment)},cwd=${JSON.stringify(cwd)},env={COWBOY_CALL_CONTEXT=""},enabled=true,tool_timeout_sec=75}`,
  ]);
}
