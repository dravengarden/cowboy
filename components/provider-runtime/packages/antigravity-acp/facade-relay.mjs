// The stdio MCP server `agy` launches for the `cowboy` facade (design R3-17).
// It only relays bytes between its stdio and the proxy's unit-private Unix
// socket; the facade itself and the one execution connection live in the
// proxy. Arguments: the socket path and this session's channel token.
import { createHash } from "node:crypto";
import { appendFileSync } from "node:fs";
import { connect } from "node:net";

const argv = process.argv.slice(2);
const [socketPath, channel] = argv;
let canaryTraceFile;
let canaryNonce;
for (let i = 2; i < argv.length; i += 1) {
  if (
    argv[i] === "--canary-trace-file" &&
    typeof argv[i + 1] === "string" &&
    argv[i + 1].length > 0
  ) {
    canaryTraceFile = argv[i + 1];
    i += 1;
  } else if (
    argv[i] === "--canary-nonce" &&
    typeof argv[i + 1] === "string" &&
    argv[i + 1].length > 0
  ) {
    canaryNonce = argv[i + 1];
    i += 1;
  }
}
if (!socketPath || !/^[a-f0-9-]{36}$/.test(channel ?? "")) {
  process.stderr.write("cowboy facade relay: invalid arguments\n");
  process.exit(2);
}

const REMINDER_RE = /<system-reminder>[\s\S]*?<\/system-reminder>/;

function verifyStdoutNote(parsed, outLine, canaryMeta) {
  const blocks = Array.isArray(parsed?.result?.content) ? parsed.result.content : [];
  const texts = blocks
    .filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text);
  let matchedNote = null;
  for (const text of texts) {
    const match = REMINDER_RE.exec(text);
    if (match) {
      matchedNote = match[0];
      break;
    }
  }
  const stdoutNoteSha256 = matchedNote
    ? createHash("sha256").update(matchedNote, "utf8").digest("hex")
    : null;
  const expected =
    typeof canaryMeta?.expectedNoteDigest === "string" ? canaryMeta.expectedNoteDigest : null;
  // R3-L1: The independent CLI `--canary-nonce` is authoritative and required
  // for a positive proof; metadata from the facade can never supply or bypass it.
  const nonceInStdout = Boolean(
    canaryNonce &&
      matchedNote &&
      matchedNote.includes(canaryNonce) &&
      outLine.includes(canaryNonce),
  );
  const stdoutNoteMatched = Boolean(
    matchedNote &&
      stdoutNoteSha256 &&
      expected &&
      stdoutNoteSha256 === expected &&
      outLine.includes("<system-reminder>") &&
      nonceInStdout,
  );
  return { stdoutNoteMatched, stdoutNoteSha256, nonceInStdout };
}

const socket = connect(socketPath);
socket.on("connect", () => {
  socket.write(`${JSON.stringify({ cowboy_channel: channel })}\n`);
  process.stdin.pipe(socket);
  if (!canaryTraceFile) {
    socket.pipe(process.stdout);
    return;
  }
  // R3-M20 / M-Proxy-8: Canary-only trace mode strips relay-private `_cowboyCanary`
  // before native stdout and derives the delivery receipt from the actual
  // serialized outgoing JSON-RPC text inside the stdout write callback.
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
      let parsed;
      try {
        parsed = JSON.parse(line);
        if (parsed && typeof parsed === "object" && Object.hasOwn(parsed, "_cowboyCanary")) {
          canaryMeta = parsed._cowboyCanary;
          delete parsed._cowboyCanary;
          outLine = JSON.stringify(parsed);
        }
      } catch {
        parsed = undefined;
      }
      const proof = canaryMeta ? verifyStdoutNote(parsed, outLine, canaryMeta) : null;
      process.stdout.write(`${outLine}\n`, (err) => {
        if (!canaryMeta) return;
        try {
          appendFileSync(
            canaryTraceFile,
            `${JSON.stringify({
              inspectId: canaryMeta.inspectId,
              noteAttached: Boolean(canaryMeta.noteAttached),
              ...proof,
              written: !err,
              ts: Date.now(),
            })}\n`,
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
