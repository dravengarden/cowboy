// Run a single stateless node:test file across N parallel OS processes and
// verify every registered top-level test ran in exactly one shard.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
let shards = 8;
const files = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === "--shards") {
    shards = Number(args[++i]);
  } else {
    files.push(args[i]);
  }
}
if (!Number.isInteger(shards) || shards < 1 || files.length !== 1) {
  console.error("usage: node tools/run-sharded-node-test.mjs [--shards N] <test-file>");
  process.exit(2);
}

const target = files[0];
const dir = mkdtempSync(join(tmpdir(), "cowboy-node-shards-"));
const start = Date.now();

function runShard(index) {
  return new Promise((resolve) => {
    const receipt = join(dir, `shard-${index}.json`);
    const child = spawn(
      process.execPath,
      [
        "--import",
        "./tools/register-memory-client.mjs",
        "--import",
        "./tools/shard-node-test.mjs",
        "--test",
        target,
      ],
      {
        env: {
          ...process.env,
          COWBOY_NODE_TEST_SHARD: `${index}/${shards}`,
          COWBOY_NODE_TEST_RECEIPT: receipt,
        },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let output = "";
    child.stdout.on("data", (chunk) => {
      output += chunk;
    });
    child.stderr.on("data", (chunk) => {
      output += chunk;
    });
    child.on("close", (code, signal) => {
      resolve({ index, code: code ?? 1, signal, receipt, output });
    });
  });
}

try {
  const results = await Promise.all(Array.from({ length: shards }, (_, index) => runShard(index)));
  let failed = false;
  let expectedRegistered = null;
  let totalSelected = 0;
  for (const result of results) {
    if (result.code !== 0) {
      failed = true;
      process.stderr.write(
        `=== shard ${result.index}/${shards} FAILED (code=${result.code}) ===\n`,
      );
      process.stderr.write(result.output);
      continue;
    }
    const receipt = JSON.parse(readFileSync(result.receipt, "utf8"));
    if (expectedRegistered === null) {
      expectedRegistered = receipt.registered;
    } else if (receipt.registered !== expectedRegistered) {
      failed = true;
      process.stderr.write(
        `shard ${result.index}/${shards} saw ${receipt.registered} tests, expected ${expectedRegistered}\n`,
      );
    }
    totalSelected += receipt.selected;
  }
  if (
    !failed &&
    (expectedRegistered === null || expectedRegistered < 1 || totalSelected !== expectedRegistered)
  ) {
    failed = true;
    process.stderr.write(
      `shard accounting mismatch: selected=${totalSelected}, registered=${expectedRegistered}\n`,
    );
  }
  const elapsed = ((Date.now() - start) / 1000).toFixed(2);
  if (failed) {
    process.exit(1);
  }
  console.log(
    `sharded ${target}: ${totalSelected} tests across ${shards} shards passed in ${elapsed}s`,
  );
} finally {
  rmSync(dir, { recursive: true, force: true });
}
