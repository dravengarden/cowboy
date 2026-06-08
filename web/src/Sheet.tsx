import type { ReactNode } from "react";
import { BottomSheet, type BottomSheetProps } from "./_shell";
import { useFrostedSheets } from "./frostedSheets";

// cowboy's BottomSheet: the shared sheet wired to the app-wide "Frosted sheets"
// preference, so every modal honours the toggle from ONE place (call sites never
// thread `frosted` themselves). Drop-in for the shared BottomSheet.
export function Sheet(props: Omit<BottomSheetProps, "frosted">): ReactNode {
  const frosted = useFrostedSheets();
  return <BottomSheet {...props} frosted={frosted} />;
}
