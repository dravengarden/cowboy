// Loader hook that redirects `node:test` to the sharded proxy when
// COWBOY_NODE_TEST_SHARD is active.
import { registerHooks } from "node:module";

const proxyUrl = new URL("./shard-node-test-proxy.mjs", import.meta.url).href;

if (process.env.COWBOY_NODE_TEST_SHARD) {
  registerHooks({
    resolve(specifier, context, next) {
      if (specifier === "node:test" && context.parentURL !== proxyUrl) {
        return { url: proxyUrl, shortCircuit: true };
      }
      return next(specifier, context);
    },
  });
}
