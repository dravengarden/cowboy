import type { ReactNode } from "react";
import { BottomSheet, type BottomSheetProps } from "./_shell";

// cowboy's BottomSheet: always the frosted 磨砂玻璃 material (no per-app toggle —
// the translucent surface is the house look). Drop-in for the shared BottomSheet.
export function Sheet(props: Omit<BottomSheetProps, "frosted">): ReactNode {
  return <BottomSheet {...props} frosted />;
}
