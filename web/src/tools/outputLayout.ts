const ANSI_ESCAPE = new RegExp(`${String.fromCharCode(27)}\\[[0-?]*[ -/]*[@-~]`, "g");

/** Terminal output whose columns carry meaning should preserve every line.
 * Natural-language output remains wrapped on touch devices. This is only an
 * initial presentation choice; Tool details always exposes both layouts. */
export function outputPrefersHorizontalScroll(text: string): boolean {
  const lines = text.replace(ANSI_ESCAPE, "").split("\n").filter((line) => line.trim().length > 0);
  if (lines.length === 0) return false;
  if (lines.some((line) => line.includes("\t"))) return true;

  const pipeRows = lines.filter((line) => (line.match(/\|/g)?.length ?? 0) >= 2).length;
  if (pipeRows >= 2) return true;

  const borderRows = lines.filter((line) => /^\s*[+|]?[-=]{3,}(?:[+|][-+=]{3,})+[+|]?\s*$/.test(line)).length;
  if (borderRows > 0) return true;

  // Fixed-width command output often uses repeated 2+ space column gaps but
  // no visible pipes (ps, systemctl, docker, kubectl). Require several rows so
  // prose containing an occasional double space does not switch to scrolling.
  const fixedColumnRows = lines.filter((line) => /\S {2,}\S/.test(line)).length;
  return fixedColumnRows >= 3 && fixedColumnRows >= Math.ceil(lines.length * 0.5);
}
