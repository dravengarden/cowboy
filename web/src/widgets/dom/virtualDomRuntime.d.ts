declare module "virtual:cowboy-dom-runtime" {
  /** The full-DOM app runtime module (React, ReactDOM, Three, Sucrase). */
  export const runtimeSource: string;
  /** The /app-shell.html document, for fixtures that serve it themselves. */
  export const shellHtml: string;
}
