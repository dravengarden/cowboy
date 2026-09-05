// Private release glue, executed by the Node runtime in this exact archive.
// codex-acp owns ACP; the declared -c arguments belong to its Codex subprocess.
import { spawn } from "node:child_process";
import { isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const cliKey = "COWBOY_PRIVATE_CODEX_EXECUTABLE";
const argsKey = "COWBOY_PRIVATE_CODEX_ARGUMENTS";

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
  if (args[0] === "--cowboy-private-cli") {
    const executable = process.env[cliKey];
    const configuration = JSON.parse(process.env[argsKey] ?? "[]");
    if (
      !executable || !isAbsolute(executable) || !Array.isArray(configuration)
    ) {
      throw new Error("Missing exact private Codex binding");
    }
    const parsed = splitConfigurationArguments(configuration);
    if (parsed.arguments.length) {
      throw new Error("Unexpected private Codex arguments");
    }
    const environment = { ...process.env };
    delete environment[cliKey];
    delete environment[argsKey];
    const child = spawn(
      executable,
      [...parsed.configuration, ...args.slice(1)],
      {
        env: environment,
        stdio: "inherit",
      },
    );
    for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
      process.on(signal, () => child.kill(signal));
    }
    const code = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => resolve(code ?? (signal ? 1 : 0)));
    });
    process.exitCode = code;
    return;
  }
  const { configuration, arguments: forwarded } = splitConfigurationArguments(
    args,
  );
  if (configuration.length) {
    const executable = process.env.CODEX_PATH;
    if (!executable || !isAbsolute(executable)) {
      throw new Error("Configured Codex requires an exact Machine-bound CLI");
    }
    process.env[cliKey] = executable;
    process.env[argsKey] = JSON.stringify(configuration);
    process.env.CODEX_PATH = fileURLToPath(
      new URL("../bin/cowboy-configured-cli", import.meta.url),
    );
  }
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
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main(process.argv.slice(2));
}
