// The stdio MCP server `agy` launches for the `cowboy` facade (design R3-17).
// It only relays bytes between its stdio and the proxy's unit-private Unix
// socket; the facade itself and the one execution connection live in the
// proxy. Arguments: the socket path and this session's channel token.
import { appendFileSync } from "node:fs";
import { connect } from "node:net";

const argv = process.argv.slice(2);
const [socketPath, channel] = argv;
let canaryTraceFile;
if (argv[2] === "--canary-trace-file" && typeof argv[3] === "string" && argv[3].length > 0) {
  canaryTraceFile = argv[3];
}
if (!socketPath || !/^[a-f0-9-]{36}$/.test(channel ?? "")) {
  process.stderr.write("cowboy facade relay: invalid arguments\n");
  process.exit(2);
}
const socket = connect(socketPath);
socket.on("connect", () => {
  socket.write(`${JSON.stringify({ cowboy_channel: channel })}\n`);
  process.stdin.pipe(socket);
  if (!canaryTraceFile) {
    socket.pipe(process.stdout);
    return;
  }
  // R3-M20: Canary-only trace mode strips relay-private `_cowboyCanary` before
  // native stdout and logs delivery proof inside the stdout write callback.
  let buffered = "";
  socket.setEncoding("utf8");
  socket.on("data", (chunk) => {
    buffered += chunk;
    let nl;
    while ((nl = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, nl);
      buffered = buffered.slice(nl + 1);
      if (!line.trim()) continue;
      let canaryMeta;
      let outLine = line;
      try {
        const parsed = JSON.parse(line);
        if (parsed && typeof parsed === "object" && Object.hasOwn(parsed, "_cowboyCanary")) {
          canaryMeta = parsed._cowboyCanary;
          delete parsed._cowboyCanary;
          outLine = JSON.stringify(parsed);
        }
      } catch {
        // Forward non-JSON bytes unchanged.
      }
      process.stdout.write(`${outLine}\n`, (err) => {
        if (!canaryMeta) return;
        try {
          appendFileSync(
            canaryTraceFile,
            `${JSON.stringify({ ...canaryMeta, written: !err, ts: Date.now() })}\n`,
          );
        } catch {
          // Best-effort canary trace log.
        }
      });
    }
  });
});
socket.on("error", (error) => {
  process.stderr.write(`cowboy facade relay: ${error.message}\n`);
  process.exit(1);
});
socket.on("close", () => process.exit(0));
process.stdin.on("end", () => socket.end());
