/** GFM table cells cannot contain a newline, so agents write `<br>` for a line
 *  break inside a cell. Raw HTML stays disabled in rendered Markdown (it is
 *  escaped and shown as literal text), so accept exactly this one tag and turn
 *  it into a Markdown hard break. Anything else remains escaped text. */

interface MdNode {
  type: string;
  value?: string;
  children?: MdNode[];
}

const LINE_BREAK_TAG = /^<br\s*\/?>$/i;

function convert(node: MdNode): void {
  const children = node.children;
  if (!children) return;
  for (let i = 0; i < children.length; i += 1) {
    const child = children[i]!;
    if (
      child.type === "html" && LINE_BREAK_TAG.test(child.value?.trim() ?? "")
    ) {
      children[i] = { type: "break" };
    } else {
      convert(child);
    }
  }
}

export default function remarkLineBreakTags() {
  return (tree: MdNode): void => convert(tree);
}
