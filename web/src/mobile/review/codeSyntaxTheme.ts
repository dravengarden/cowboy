export interface CodeSyntaxPalette {
  foreground: string;
  muted: string;
  keyword: string;
  string: string;
  number: string;
  type: string;
  function: string;
  property: string;
  constant: string;
  operator: string;
  tag: string;
  invalid: string;
}

// Code Review is a reading surface, so token colours favour sustained
// contrast over editor-theme novelty. Warm red is reserved for invalid syntax
// and diff removals; ordinary strings stay green in both colour schemes.
export function codeSyntaxPalette(mode: "light" | "dark"): CodeSyntaxPalette {
  return mode === "dark"
    ? {
      foreground: "#ede9fe",
      muted: "#94a3b8",
      keyword: "#c4b5fd",
      string: "#86efac",
      number: "#fdba74",
      type: "#7dd3fc",
      function: "#f9a8d4",
      property: "#a5b4fc",
      constant: "#fde68a",
      operator: "#cbd5e1",
      tag: "#fda4af",
      invalid: "#fb7185",
    }
    : {
      foreground: "#252131",
      muted: "#596579",
      keyword: "#5b21b6",
      string: "#166534",
      number: "#9a3412",
      type: "#075985",
      function: "#9d174d",
      property: "#3730a3",
      constant: "#854d0e",
      operator: "#475569",
      tag: "#be123c",
      invalid: "#b91c1c",
    };
}
