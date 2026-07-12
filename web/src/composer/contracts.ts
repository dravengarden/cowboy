import type { Status } from "../protocol";

export type ComposerVariant = "overlay" | "column";

export interface ComposerWorkspaceProps {
  sessionId: string;
  status: Status;
  variant?: ComposerVariant;
  surface?: "desktop" | "mobile";
}
