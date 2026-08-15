/**
 * An HTTP session bootstrap is a snapshot taken at an unknown point in time.
 * A live WebSocket config event received after the request began is newer than
 * that snapshot, so applying the bootstrap would roll the controls backwards.
 */
export function shouldApplyHydratedConfigOptions(
  revisionAtRequestStart: number,
  currentRevision: number,
): boolean {
  return revisionAtRequestStart === currentRevision;
}
