import { HighlightStyle } from "@codemirror/language";
import { tags } from "@lezer/highlight";
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

/** Review's syntax colours for CodeMirror, shared by Mobile and Desktop. */
export function reviewHighlightStyle(mode: "light" | "dark"): HighlightStyle {
  const syntax = codeSyntaxPalette(mode);
  return HighlightStyle.define([
    { tag: tags.keyword, color: syntax.keyword, fontWeight: "600" },
    {
      tag: [tags.string, tags.special(tags.string), tags.regexp, tags.escape],
      color: syntax.string,
    },
    {
      tag: [tags.number, tags.bool, tags.null],
      color: syntax.number,
    },
    {
      tag: [tags.typeName, tags.className, tags.namespace],
      color: syntax.type,
    },
    {
      tag: [tags.function(tags.variableName), tags.function(tags.propertyName)],
      color: syntax.function,
    },
    {
      tag: [tags.propertyName, tags.attributeName, tags.labelName],
      color: syntax.property,
    },
    {
      tag: [tags.constant(tags.name), tags.standard(tags.name), tags.local(tags.name)],
      color: syntax.constant,
    },
    {
      tag: [tags.operator, tags.punctuation, tags.bracket],
      color: syntax.operator,
    },
    {
      tag: [tags.comment, tags.docComment, tags.meta],
      color: syntax.muted,
      fontStyle: "italic",
    },
    { tag: [tags.tagName, tags.heading], color: syntax.tag },
    {
      tag: [tags.variableName, tags.name],
      color: syntax.foreground,
    },
    {
      tag: tags.invalid,
      color: syntax.invalid,
      textDecoration: "underline wavy",
    },
  ]);
}
