// Source-test package resolution. Released Plugins contain their pinned copy.
import { registerHooks } from "node:module";
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "@cowboy/memory-client") {
      return {
        url: new URL("../components/memory-client/index.mjs", import.meta.url)
          .href,
        shortCircuit: true,
      };
    }
    return next(specifier, context);
  },
});
