export interface EmbeddedFormatRequest {
  language: string;
  source: string;
  columns: number;
}

type EmbeddedFormatter = (source: string, columns: number) => Promise<string>;

// Embedded formatting is a progressive display enhancement. Keep both the
// registry and its resource limits independent from shell discovery so a new
// language never changes Bash parsing, source bytes, or copy behaviour.
const MAX_FORMAT_RUNES = 16 * 1024;

async function formatJavaScript(source: string, columns: number, typescript: boolean): Promise<string> {
  const [{ format }, estree, parser] = await Promise.all([
    import("prettier/standalone"),
    import("prettier/plugins/estree"),
    typescript ? import("prettier/plugins/typescript") : import("prettier/plugins/babel"),
  ]);
  return (await format(source, {
    parser: typescript ? "typescript" : "babel",
    plugins: [parser, estree],
    printWidth: Math.max(40, columns - 6),
    semi: true,
  })).trimEnd();
}

const formatters: Readonly<Record<string, EmbeddedFormatter>> = {
  javascript: (source, columns) => formatJavaScript(source, columns, false),
  typescript: (source, columns) => formatJavaScript(source, columns, true),
};

/** Format only a parser-identified embedded payload. Any unsupported language,
 * oversized input, loader failure, or syntax error returns the decoded source
 * unchanged, allowing callers to keep the useful language highlight without
 * coupling rendering correctness to a formatter. */
export async function formatEmbeddedSource(
  { language, source, columns }: EmbeddedFormatRequest,
): Promise<string> {
  const formatter = formatters[language];
  if (!formatter || [...source].length > MAX_FORMAT_RUNES) return source;
  try {
    return await formatter(source, columns);
  } catch {
    return source;
  }
}
