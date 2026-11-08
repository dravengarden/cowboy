// A bounded startup-only command over the existing authenticated executor.
// Unknown starts retain their original ID for cleanup and are never replayed.
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";
import { Connection } from "./execution-connection.mjs";

export async function probeExecutionCli(descriptor, argv) {
  if (descriptor.binding.managed) throw new Error("Managed MCP probing is disabled");
  const connection = await Connection.open(descriptor, {
    clientName: "cowboy-mcp-probe",
    requestTimeoutMs: 10000,
  });
  const processId = "cowboy-mcp-probe-" + randomUUID();
  let closed = false;
  let submitted = false;
  try {
    if (connection.managed) throw new Error("Managed MCP probing is disabled");
    connection.assertAgentCli(argv[0]);
    submitted = true;
    const started = await connection.call("process/start", {
      processId,
      argv,
      cwd: pathToFileURL(descriptor.binding.workspace.cwd).href,
      env: {},
      tty: false,
      pipeStdin: false,
      arg0: null,
    });
    if (started.processId !== processId) throw new Error("MCP probe identity changed");
    const deadline = Date.now() + 10000;
    const chunks = [];
    let bytes = 0;
    let afterSeq = null;
    while (Date.now() < deadline) {
      connection.requestTimeoutMs = Math.max(1, deadline - Date.now());
      const result = await connection.call("process/read", {
        processId,
        afterSeq,
        maxBytes: 4097 - bytes,
        waitMs: Math.min(1000, connection.requestTimeoutMs),
      });
      if (!Array.isArray(result.chunks)) throw new Error("Invalid MCP probe output");
      for (const chunk of result.chunks) {
        if (
          !Number.isSafeInteger(chunk.seq) ||
          chunk.seq < 0 ||
          (afterSeq !== null && chunk.seq <= afterSeq) ||
          !["stdout", "stderr"].includes(chunk.stream) ||
          typeof chunk.chunk !== "string"
        )
          throw new Error("Invalid MCP probe output");
        afterSeq = chunk.seq;
        const buffer = Buffer.from(chunk.chunk, "base64");
        bytes += buffer.length;
        if (bytes > 4096) throw new Error("MCP probe output exceeded its limit");
        if (chunk.stream === "stdout") chunks.push(buffer);
      }
      if (result.closed === true && result.chunks.length === 0) {
        closed = true;
        return {
          exited: result.exited === true,
          exitCode: result.exitCode,
          output: Buffer.concat(chunks).toString("utf8"),
        };
      }
    }
    throw new Error("MCP probe timed out");
  } finally {
    if (submitted && !closed) {
      connection.requestTimeoutMs = 2000;
      await connection.call("process/terminate", { processId }).catch(() => {});
    }
    connection.close();
  }
}

// Recheck target identity at native launch without starting another probe process.
export async function verifyExecutionCli(descriptor, command) {
  if (!descriptor || descriptor.binding.managed)
    throw new Error("MCP requires bound ordinary execution");
  const connection = await Connection.open(descriptor, {
    clientName: "cowboy-mcp-launch",
    requestTimeoutMs: 10000,
  });
  try {
    return connection.assertAgentCli(command);
  } finally {
    connection.close();
  }
}
