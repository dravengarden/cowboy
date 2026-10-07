// Remark plugin sets for the transcript Markdown renderer. Front matter is a
// Markdown-file convention: chat prose must not parse it, because a reply
// shaped `---\n\ntext\n\n---` is valid YAML front matter and would vanish.

import remarkFrontmatter from "remark-frontmatter";
import remarkGfm from "remark-gfm";
import remarkMath from "remark-math";
import remarkLineBreakTags from "./markdownLineBreaks";

const PROSE = [remarkGfm, remarkMath, remarkLineBreakTags];
const DOCUMENT = [remarkFrontmatter, ...PROSE];

export function markdownRemarkPlugins(frontmatter: boolean): typeof DOCUMENT {
  return frontmatter ? DOCUMENT : PROSE;
}
