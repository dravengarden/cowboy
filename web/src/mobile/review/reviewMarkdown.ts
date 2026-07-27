const MARKDOWN_EXTENSION = /\.(?:md|mdx|markdown|mdown|mkd)$/i;

export function isMarkdownReviewPath(path: string): boolean {
  return MARKDOWN_EXTENSION.test(path);
}
