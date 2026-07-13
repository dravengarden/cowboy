const MODULE_LOAD_PATTERNS =
  /(?:importing a module script failed|failed to fetch dynamically imported module|error loading dynamically imported module|chunkloaderror|loading chunk \S+ failed)/i;

export function isModuleLoadError(error: Error): boolean {
  return MODULE_LOAD_PATTERNS.test(`${error.name} ${error.message}`);
}
