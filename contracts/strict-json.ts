// JSON.parse alone loses duplicate object keys. No JSON5, external references,
// invalid Unicode, unbounded nesting, or unbounded input at this trust boundary.
export function strictJson(
  raw: string,
  maxBytes: number,
  maxDepth: number,
): unknown {
  const fail = (): never => {
    throw new Error("invalid_json");
  };
  if (
    raw.length > maxBytes || new TextEncoder().encode(raw).length > maxBytes
  ) fail();
  let pos = 0;
  const space = (): void => {
    while (/[ \r\n\t]/.test(raw[pos] ?? "x")) pos++;
  };
  const string = (): string => {
    const start = pos++;
    while (pos < raw.length) {
      if (raw[pos] === "\\") pos += 2;
      else if (raw[pos++] === '"') {
        let value: string;
        try {
          value = JSON.parse(raw.slice(start, pos));
        } catch {
          return fail();
        }
        for (let index = 0; index < value.length; index++) {
          const unit = value.charCodeAt(index);
          if (unit >= 0xd800 && unit <= 0xdbff) {
            const low = value.charCodeAt(++index);
            if (!(low >= 0xdc00 && low <= 0xdfff)) fail();
          } else if (unit >= 0xdc00 && unit <= 0xdfff) fail();
        }
        return value;
      }
    }
    return fail();
  };
  const value = (depth: number): unknown => {
    if (depth > maxDepth) fail();
    space();
    const token = raw[pos];
    if (token === '"') return string();
    if (token === "{" || token === "[") {
      const object = token === "{";
      const end = object ? "}" : "]";
      const result: Record<string, unknown> = Object.create(null);
      const items: unknown[] = [];
      pos++;
      space();
      if (raw[pos] === end) {
        pos++;
        return object ? result : items;
      }
      while (pos < raw.length) {
        if (object) {
          if (raw[pos] !== '"') fail();
          const key = string();
          space();
          if (raw[pos++] !== ":" || Object.hasOwn(result, key)) fail();
          result[key] = value(depth + 1);
        } else items.push(value(depth + 1));
        space();
        if (raw[pos] === end) {
          pos++;
          return object ? result : items;
        }
        if (raw[pos++] !== ",") fail();
        space();
      }
      return fail();
    }
    const literal =
      /^(?:null|true|false|-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)/
        .exec(raw.slice(pos));
    if (!literal) return fail();
    pos += literal[0].length;
    const decoded: unknown = JSON.parse(literal[0]);
    if (
      typeof decoded === "number" &&
      (!Number.isFinite(decoded) || Math.abs(decoded) > Number.MAX_SAFE_INTEGER)
    ) fail();
    return decoded;
  };
  const result = value(0);
  space();
  if (pos !== raw.length) fail();
  return result;
}
