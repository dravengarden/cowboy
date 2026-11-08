// The side-by-side layout of a decoded patch: two documents of equal length
// whose rows line up. Context sits on both sides; a run of deletions faces
// the additions that follow it, and the shorter side is padded with filler
// rows. Each row remembers its patch row, so selections, hunk motions and
// Inspector anchors keep meaning the unified patch's coordinates.

import type { DecodedPatch } from "./unifiedPatchDecoder";

export type SplitRowKind = "context" | "add" | "delete" | "filler" | "hunk" | "meta";

export interface SplitSide {
  readonly text: string;
  readonly kinds: readonly SplitRowKind[];
  /** File line shown in the gutter, or "" for headers and filler. */
  readonly labels: readonly string[];
}

export interface SplitDiff {
  readonly left: SplitSide;
  readonly right: SplitSide;
  /** One-based unified patch row of each split row (the row it stands for). */
  readonly unifiedRows: readonly number[];
}

interface Row {
  left: { text: string; kind: SplitRowKind; label: string };
  right: { text: string; kind: SplitRowKind; label: string };
  unified: number;
}

const filler = { text: "", kind: "filler" as const, label: "" };

export function splitDiff(patch: DecodedPatch): SplitDiff {
  const rows: Row[] = [];
  if (patch.binary) {
    const text = "Binary file: no text to compare";
    rows.push({ left: { text, kind: "meta", label: "" }, right: { text, kind: "meta", label: "" }, unified: 1 });
  } else if (patch.renameFrom !== undefined && patch.hunks.length === 0) {
    rows.push({
      left: { text: `renamed from ${patch.renameFrom}`, kind: "meta", label: "" },
      right: { text: `renamed to ${patch.renameTo ?? ""}`, kind: "meta", label: "" },
      unified: 1,
    });
  }
  for (const hunk of patch.hunks) {
    rows.push({
      left: { text: hunk.header, kind: "hunk", label: "" },
      right: { text: hunk.header, kind: "hunk", label: "" },
      unified: hunk.row,
    });
    let deletes: typeof hunk.lines = [];
    let adds: typeof hunk.lines = [];
    const flush = (): void => {
      const count = Math.max(deletes.length, adds.length);
      for (let index = 0; index < count; index++) {
        const old = deletes[index];
        const next = adds[index];
        rows.push({
          left: old ? { text: old.text, kind: "delete", label: String(old.oldLine) } : filler,
          right: next ? { text: next.text, kind: "add", label: String(next.newLine) } : filler,
          unified: (next ?? old)!.row,
        });
      }
      deletes = [];
      adds = [];
    };
    for (const line of hunk.lines) {
      if (line.kind === "delete") {
        if (adds.length > 0) flush();
        deletes.push(line);
      } else if (line.kind === "add") {
        adds.push(line);
      } else {
        flush();
        rows.push({
          left: { text: line.text, kind: "context", label: String(line.oldLine) },
          right: { text: line.text, kind: "context", label: String(line.newLine) },
          unified: line.row,
        });
      }
    }
    flush();
  }
  const side = (pick: (row: Row) => Row["left"]): SplitSide => ({
    text: rows.map((row) => pick(row).text).join("\n"),
    kinds: rows.map((row) => pick(row).kind),
    labels: rows.map((row) => pick(row).label),
  });
  return { left: side((row) => row.left), right: side((row) => row.right), unifiedRows: rows.map((row) => row.unified) };
}

/** The split row standing for unified `row`, or the nearest one before it. */
export function splitRowOf(split: SplitDiff, row: number): number {
  let best = 1;
  split.unifiedRows.forEach((unified, index) => {
    if (unified <= row) best = index + 1;
  });
  return best;
}
