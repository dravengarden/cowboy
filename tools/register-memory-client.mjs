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
    if (specifier === "ws") {
      return next(specifier, {
        ...context,
        parentURL: new URL(
          "../dist/claude-source-tests/package.json",
          import.meta.url,
        ).href,
      });
    }
    return next(specifier, context);
  },
});
