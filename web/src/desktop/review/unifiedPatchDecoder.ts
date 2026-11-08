// A unified-diff decoder that takes the patch in arbitrary chunks
// (docs/desktop-code-review.md "Split view"). A chunk may end inside a line
// or a hunk, as a 256 KiB diff page does; the decoder buffers the partial
// line and keeps its hunk state, so the result does not depend on where the
// pages were cut. File metadata (rename, binary) is kept apart from hunks.

export type PatchLineKind = "context" | "add" | "delete";

export interface PatchLine {
  readonly kind: PatchLineKind;
  readonly text: string;
  /** One-based line in the old / new file; absent on the side it is not on. */
  readonly oldLine?: number;
  readonly newLine?: number;
  /** The line was followed by `\ No newline at end of file`. */
  readonly noNewline?: boolean;
  /** One-based row of this line in the unified patch text. */
  readonly row: number;
}

export interface PatchHunk {
  readonly header: string;
  readonly row: number;
  readonly oldStart: number;
  readonly newStart: number;
  readonly lines: PatchLine[];
}

export interface DecodedPatch {
  readonly headers: readonly string[];
  readonly renameFrom?: string;
  readonly renameTo?: string;
  readonly binary: boolean;
  readonly hunks: readonly PatchHunk[];
  /** Lines ended with CRLF; `\r` is removed from `text`. */
  readonly crlf: boolean;
  /** The patch was cut short by its source. */
  readonly truncated: boolean;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

export interface PatchDecoder {
  push(chunk: string): void;
  finish(truncated?: boolean): DecodedPatch;
}

export function createPatchDecoder(): PatchDecoder {
  let pending = "";
  let row = 0;
  const headers: string[] = [];
  const hunks: PatchHunk[] = [];
  let hunk: PatchHunk | undefined;
  let oldLine = 0;
  let newLine = 0;
  let renameFrom: string | undefined;
  let renameTo: string | undefined;
  let binary = false;
  let crlf = false;
  const line = (raw: string): void => {
    row += 1;
    const text = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
    if (text !== raw) crlf = true;
    const header = HUNK.exec(text);
    if (header) {
      oldLine = Number(header[1]);
      newLine = Number(header[2]);
      hunk = { header: text, row, oldStart: oldLine, newStart: newLine, lines: [] };
      hunks.push(hunk);
      return;
    }
    if (text.startsWith("\\")) {
      const last = hunk?.lines.at(-1);
      if (hunk && last) hunk.lines[hunk.lines.length - 1] = { ...last, noNewline: true };
      return;
    }
    if (!hunk) {
      headers.push(text);
      if (text.startsWith("rename from ")) renameFrom = text.slice(12);
      else if (text.startsWith("rename to ")) renameTo = text.slice(10);
      else if (text.startsWith("Binary files ") || text === "GIT binary patch") binary = true;
      return;
    }
    if (text.startsWith("+")) {
      hunk.lines.push({ kind: "add", text: text.slice(1), newLine: newLine++, row });
    } else if (text.startsWith("-")) {
      hunk.lines.push({ kind: "delete", text: text.slice(1), oldLine: oldLine++, row });
    } else if (text.startsWith(" ") || text === "") {
      // A blank line inside a hunk is an empty context line whose leading
      // space a tool trimmed.
      hunk.lines.push({ kind: "context", text: text.slice(1), oldLine: oldLine++, newLine: newLine++, row });
    } else {
      // The next file's header ends this hunk.
      hunk = undefined;
      headers.push(text);
    }
  };
  return {
    push(chunk) {
      const parts = (pending + chunk).split("\n");
      pending = parts.pop() ?? "";
      for (const part of parts) line(part);
    },
    finish(truncated = false) {
      if (pending) line(pending);
      pending = "";
      return {
        headers,
        ...(renameFrom === undefined ? {} : { renameFrom }),
        ...(renameTo === undefined ? {} : { renameTo }),
        binary,
        hunks,
        crlf,
        truncated,
      };
    },
  };
}

export function decodePatch(text: string, truncated = false): DecodedPatch {
  const decoder = createPatchDecoder();
  decoder.push(text);
  return decoder.finish(truncated);
}
