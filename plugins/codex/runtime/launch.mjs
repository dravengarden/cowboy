// This Provider owns both argument parsing and its patched native RPC client.
// The client spawns the exact CODEX_PATH directly, including these -c options.
import { isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export function splitConfigurationArguments(args) {
  const configuration = [];
  let index = 0;
  while (args[index] === "-c" || args[index] === "--config") {
    const value = args[index + 1];
    if (typeof value !== "string" || !/^[a-zA-Z0-9_.-]+=/.test(value)) {
      throw new Error("Invalid private Codex configuration argument");
    }
    configuration.push("-c", value);
    index += 2;
  }
  return { configuration, arguments: args.slice(index) };
}

export async function main(args) {
  const { configuration, arguments: forwarded } = splitConfigurationArguments(
    args,
  );
  if (configuration.length && !isAbsolute(process.env.CODEX_PATH ?? "")) {
    throw new Error("Configured Codex requires an exact Machine-bound CLI");
  }
  process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS = JSON.stringify(configuration);
  const upstream = fileURLToPath(
    new URL(
      "./node_modules/@agentclientprotocol/codex-acp/dist/index.js",
      import.meta.url,
    ),
  );
  process.argv = [process.execPath, upstream, ...forwarded];
  await import(upstream);
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main(process.argv.slice(2));
}
