/**
 * Two-glyph identity for a session tile in the collapsed Sessions rail.
 *
 * Most sessions share a provider icon, so the icon alone cannot tell rows
 * apart. Like collapsed workspace switchers, the rail uses the initials of the
 * first two title words, or the first two glyphs of a single word. Glyphs are
 * grapheme-safe so CJK titles and emoji are never split.
 */
export function sessionMonogram(title: string): string {
  const words = title
    .split(/[\s\-_/·.:]+/u)
    .map((word) => word.replace(/^[^\p{L}\p{N}]+/u, ""))
    .filter(Boolean);
  const glyphs = (text: string): string[] => {
    const Segmenter = (Intl as { Segmenter?: typeof Intl.Segmenter }).Segmenter;
    return Segmenter
      ? [...new Segmenter(undefined, { granularity: "grapheme" }).segment(text)]
        .map((part) => part.segment)
      : Array.from(text);
  };
  const first = words[0];
  if (!first) return "?";
  const second = words[1];
  const letters = second
    ? [glyphs(first)[0] ?? "", glyphs(second)[0] ?? ""]
    : glyphs(first).slice(0, 2);
  const cased = letters.join("");
  // Latin initials read as a badge in upper case; CJK has no case to change.
  return second ? cased.toUpperCase() : (letters[0] ?? "").toUpperCase() + (letters[1] ?? "");
}
