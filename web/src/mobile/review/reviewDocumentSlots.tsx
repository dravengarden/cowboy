import { ChevronLeft, ChevronRight } from "@mui/icons-material";
import { Box, IconButton, Typography } from "@mui/material";
import { createContext, useContext, useMemo, useState } from "react";

/**
 * Places in the Review file controls where an open diff puts its own controls,
 * so line counts and hunk navigation share the file pager's row instead of
 * adding a row above it. Without a provider the document keeps its own row.
 */
export interface ReviewDocumentSlots {
  /** Under the `n / total` file position. */
  readonly counts: HTMLElement | null;
  /** Beside the file pager. */
  readonly hunks: HTMLElement | null;
  /** Inside the More actions sheet while it is open. */
  readonly actions: HTMLElement | null;
}

export const ReviewDocumentSlotsContext = createContext<
  ReviewDocumentSlots | undefined
>(undefined);

type SlotName = keyof ReviewDocumentSlots | "inlineActions";
type SlotNodes = Record<SlotName, HTMLElement | null>;

/** Mounted Review control containers and a stable ref callback for each. */
export function useReviewSlotContainers(): [
  SlotNodes,
  Record<SlotName, (node: HTMLElement | null) => void>,
] {
  const [nodes, setNodes] = useState<SlotNodes>({
    counts: null,
    hunks: null,
    actions: null,
    inlineActions: null,
  });
  const refs = useMemo(() => {
    const set = (name: SlotName) => (node: HTMLElement | null) =>
      setNodes((current) =>
        current[name] === node ? current : { ...current, [name]: node }
      );
    return {
      counts: set("counts"),
      hunks: set("hunks"),
      actions: set("actions"),
      inlineActions: set("inlineActions"),
    };
  }, []);
  return [nodes, refs];
}

/** The slots, or undefined unless both in-row targets are mounted. */
export function useReviewDocumentSlots(): ReviewDocumentSlots | undefined {
  const slots = useContext(ReviewDocumentSlotsContext);
  return slots?.counts && slots.hunks ? slots : undefined;
}

/** Previous/next changed file, with the diff's line counts beneath. */
export function ReviewDiffPager({
  index,
  total,
  onMove,
  countsRef,
  hunksRef,
}: {
  index: number;
  total: number;
  onMove: (offset: number) => void;
  countsRef: (node: HTMLElement | null) => void;
  hunksRef: (node: HTMLElement | null) => void;
}): React.JSX.Element {
  return (
    <>
      <Box sx={{ flex: 1 }} />
      <IconButton
        aria-label="Previous change"
        disabled={index <= 0}
        onClick={() => onMove(-1)}
      >
        <ChevronLeft />
      </IconButton>
      <Box
        sx={{
          minWidth: 52,
          display: "flex",
          flexDirection: "column",
          alignItems: "center",
          lineHeight: 1.2,
        }}
      >
        <Typography variant="caption" color="text.secondary">
          {index + 1} / {total}
        </Typography>
        <Box
          ref={countsRef}
          data-review-diff-counts
          sx={{ display: "flex", gap: 0.5, "&:empty": { display: "none" } }}
        />
      </Box>
      <IconButton
        aria-label="Next change"
        disabled={index >= total - 1}
        onClick={() => onMove(1)}
      >
        <ChevronRight />
      </IconButton>
      <Box ref={hunksRef} data-review-diff-hunks sx={{ display: "contents" }} />
    </>
  );
}
