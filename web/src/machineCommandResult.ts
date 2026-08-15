export type MachineCommandResultPresentation = {
  severity: "success" | "warning";
  message: string;
};

/**
 * Machine command details are diagnostics from a separately versioned peer.
 * They can contain decoder internals, paths, or dependency names, so the
 * ordinary Machine card must never render them verbatim.
 */
export function machineCommandResultPresentation(
  accepted: boolean,
): MachineCommandResultPresentation {
  return accepted ? { severity: "success", message: "Command accepted" } : {
    severity: "warning",
    message:
      "The Machine rejected this command. Refresh its inventory and retry after updating the Machine.",
  };
}
