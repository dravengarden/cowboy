import type { ConfigOption } from "../protocol";

// Presentation of Provider config options shared by the Top bar summary and
// the Run configuration dialog.

export function optionLabel(option: ConfigOption): string {
  const name = option.name.toLowerCase();
  if (option.id === "mode") return "Agent mode";
  if (option.id === "session_mode") return "Session mode";
  if (option.id === "permission_mode") return "Permission";
  if (option.id === "model") return "Model";
  if (name.includes("reasoning") && name.includes("effort")) {
    return "Reasoning effort";
  }
  if (name.includes("fast")) return "Fast mode";
  return option.name;
}

export function optionShortcut(option: ConfigOption): string | undefined {
  const label = optionLabel(option);
  if (label === "Agent mode") return "A";
  if (label === "Session mode") return "A";
  if (label === "Permission") return "P";
  if (label === "Model") return "M";
  if (label === "Reasoning effort") return "E";
  if (label === "Collaboration mode") return "C";
  if (label === "Fast mode") return "F";
  return undefined;
}

export function currentOptionName(option: ConfigOption): string {
  return option.options.find((candidate) =>
    String(candidate.value) === String(option.currentValue)
  )?.name ??
    String(option.currentValue);
}

export function compactOptionName(option: ConfigOption): string {
  const current = currentOptionName(option);
  const label = optionLabel(option);
  if (current === "Agent (full access)") return "Full access";
  if (label === "Fast mode") return `Fast ${current.toLowerCase()}`;
  return current;
}
