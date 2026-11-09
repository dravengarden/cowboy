declare module "virtual:cowboy-app-runtime" {
  /** The app realm bundle evaluated inside QuickJS. */
  export const vmSource: string;
  /** The trusted runtime Worker. */
  export const workerSource: string;
}
