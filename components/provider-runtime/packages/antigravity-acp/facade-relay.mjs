// The stdio MCP server `agy` launches for the `cowboy` facade (design R3-17).
// It only relays bytes between its stdio and the proxy's unit-private Unix
// socket; the facade itself and the one execution connection live in the
// proxy. Arguments: the socket path and this session's channel token.
import { connect } from "node:net";

const [socketPath, channel] = process.argv.slice(2);
if (!socketPath || !/^[a-f0-9-]{36}$/.test(channel ?? "")) {
  process.stderr.write("cowboy facade relay: invalid arguments\n");
  process.exit(2);
}
const socket = connect(socketPath);
socket.on("connect", () => {
  socket.write(`${JSON.stringify({ cowboy_channel: channel })}\n`);
  process.stdin.pipe(socket);
  socket.pipe(process.stdout);
});
socket.on("error", (error) => {
  process.stderr.write(`cowboy facade relay: ${error.message}\n`);
  process.exit(1);
});
socket.on("close", () => process.exit(0));
process.stdin.on("end", () => socket.end());
