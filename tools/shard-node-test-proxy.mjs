// Sharded proxy for `node:test` used by tools/run-sharded-node-test.mjs to
// parallelize stateless test files across OS processes without editing files
// whose source tree is hashed by a Plugin component digest.
import { writeFileSync } from "node:fs";

const real = process.getBuiltinModule("node:test");
const spec = process.env.COWBOY_NODE_TEST_SHARD ?? "";
const match = /^(\d+)\/(\d+)$/.exec(spec);
if (!match) {
  throw new Error(`invalid COWBOY_NODE_TEST_SHARD: ${JSON.stringify(spec)}`);
}
const shardIndex = Number(match[1]);
const shardTotal = Number(match[2]);
if (
  !Number.isInteger(shardIndex) ||
  !Number.isInteger(shardTotal) ||
  shardTotal < 1 ||
  shardIndex >= shardTotal
) {
  throw new Error(`out-of-range COWBOY_NODE_TEST_SHARD: ${spec}`);
}

let registered = 0;
let selected = 0;

function shardedTest(...args) {
  const index = registered++;
  if (index % shardTotal !== shardIndex) return Promise.resolve();
  selected += 1;
  return real.test(...args);
}

for (const key of Object.keys(real.test)) {
  shardedTest[key] = real.test[key];
}

const receiptPath = process.env.COWBOY_NODE_TEST_RECEIPT;
if (receiptPath) {
  process.on("exit", () => {
    writeFileSync(receiptPath, JSON.stringify({ shardIndex, shardTotal, registered, selected }));
  });
}

export default shardedTest;
export const test = shardedTest;
export const it = shardedTest;
export const {
  after,
  afterEach,
  before,
  beforeEach,
  describe,
  mock,
  only,
  run,
  skip,
  snapshot,
  suite,
  todo,
} = real;
