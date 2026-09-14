// Development gate: compare the actual Rust CLI and TS linker on identical
// bounded, synthetic input. No Catalog, auth, Machine, network or live state.
import {
  checkComposition,
  type CheckedStructure,
  type CompositionCheckCode,
  CompositionCheckError,
} from "../contracts/composition-check.ts";
import { linkVectors } from "../contracts/composition.fixtures.ts";

if (Deno.args.length !== 1) {
  throw new Error("expected the freshly built Cowboy CLI path");
}
const executable = await Deno.realPath(Deno.args[0]);
const directory = await Deno.makeTempDir({
  prefix: "cowboy-composition-links-",
});
const decoder = new TextDecoder("utf-8", { fatal: true });
const vectors = linkVectors();
let accepted = 0;
let refused = 0;
try {
  for (const vector of vectors) {
    const path = `${directory}/proposal.json`;
    await Deno.writeTextFile(path, vector.raw, { mode: 0o600 });
    const cancellation = new AbortController();
    const timeout = setTimeout(() => cancellation.abort(), 5_000);
    let result: Deno.CommandOutput;
    try {
      result = await new Deno.Command(executable, {
        args: ["composition", "check", path],
        clearEnv: true,
        stdin: "null",
        stdout: "piped",
        stderr: "piped",
        signal: cancellation.signal,
      }).output();
    } finally {
      clearTimeout(timeout);
    }
    let report: CheckedStructure | undefined;
    let code: CompositionCheckCode | undefined;
    try {
      report = await checkComposition(vector.raw);
    } catch (error) {
      if (!(error instanceof CompositionCheckError)) throw error;
      code = error.code;
    }
    if (
      code !== vector.error || result.success !== (vector.error === undefined)
    ) {
      throw new Error(`link acceptance differs: ${vector.name}`);
    }
    if (vector.error) {
      if (
        result.stdout.length ||
        decoder.decode(result.stderr).trim() !== `Error: ${vector.error}`
      ) {
        throw new Error(`closed error differs: ${vector.name}`);
      }
      refused++;
    } else {
      // Compare every field, including canonical digest, site/edge projections,
      // stable tie ordering and reverse dependencies, not just acceptance.
      if (
        result.stderr.length ||
        JSON.stringify(JSON.parse(decoder.decode(result.stdout))) !==
          JSON.stringify(report)
      ) {
        throw new Error(`link report differs: ${vector.name}`);
      }
      accepted++;
    }
  }
} finally {
  // Only this gate's freshly allocated, private fixture directory is removed.
  await Deno.remove(directory, { recursive: true });
}
console.log(
  JSON.stringify({
    accepted: true,
    cases: vectors.length,
    valid: accepted,
    refused,
    authorized: false,
  }),
);
