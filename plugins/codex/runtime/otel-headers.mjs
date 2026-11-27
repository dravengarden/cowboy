// Dynamic `otel.headers_helper` invoked by Codex to authenticate loopback OTLP
// log batches without exposing the token in CLI argv or on disk.
import { pathToFileURL } from "node:url";
import { CODEX_OBSERVATION_ENV, parseObservationBinding } from "./otel-observation.mjs";

export function formatObservationHeaders(env = process.env) {
  const binding = parseObservationBinding(env[CODEX_OBSERVATION_ENV]);
  if (!binding) return null;
  return `${JSON.stringify({ [binding.header_name]: binding.header_value })}\n`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const output = formatObservationHeaders(process.env);
  if (output === null) {
    process.stdout.write("{}\n");
    process.exit(1);
  }
  process.stdout.write(output);
}
