import { assertEquals } from "jsr:@std/assert";
import { EditorSelection, EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { Highlight } from "../composerHighlight";
import {
  obsidianAutoPair,
  obsidianInsertBracket,
  obsidianWrapSelection,
} from "./obsidianAutoPair";

// `|` marks a caret; `<` and `>` mark a forward selection's anchor and head.
function stateOf(marked: string): EditorState {
  const caret = marked.indexOf("|");
  let doc = marked;
  let selection: EditorSelection;
  if (caret >= 0) {
    doc = marked.replace("|", "");
    selection = EditorSelection.single(caret);
  } else {
    const anchor = marked.indexOf("<");
    doc = marked.replace("<", "");
    const head = doc.indexOf(">");
    doc = doc.replace(">", "");
    selection = EditorSelection.single(anchor, head);
  }
  const state = EditorState.create({
    doc,
    selection,
    extensions: [
      markdown({ base: markdownLanguage, codeLanguages: [], extensions: [Highlight] }),
      obsidianAutoPair,
    ],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return state;
}

function show(state: EditorState): string {
  const doc = state.doc.toString();
  const { anchor, head } = state.selection.main;
  if (anchor === head) return doc.slice(0, head) + "|" + doc.slice(head);
  const from = Math.min(anchor, head);
  const to = Math.max(anchor, head);
  return doc.slice(0, from) + "<" + doc.slice(from, to) + ">" + doc.slice(to);
}

// Types one string the way CM6's input handler would see it: Obsidian's
// transaction when it claims the input, otherwise a literal insertion.
function type(state: EditorState, text: string): EditorState {
  const claimed = obsidianInsertBracket(state, text) ??
    obsidianWrapSelection(state, text);
  const main = state.selection.main;
  const next = claimed?.state ??
    state.update({
      changes: { from: main.from, to: main.to, insert: text },
      selection: { anchor: main.from + text.length },
      userEvent: "input.type",
    }).state;
  ensureSyntaxTree(next, next.doc.length, 5000);
  return next;
}

function typeAll(marked: string, keys: string): string {
  let state = stateOf(marked);
  for (const key of keys) state = type(state, key);
  return show(state);
}

Deno.test("typing bold markers never leaves stray asterisks", () => {
  assertEquals(typeAll("|", "*"), "*|*");
  assertEquals(typeAll("|", "**"), "**|");
  assertEquals(typeAll("|", "**bold**"), "**bold**|");
  assertEquals(typeAll("foo |", "**bold**"), "foo **bold**|");
  assertEquals(typeAll("中文 |", "**bold**"), "中文 **bold**|");
  assertEquals(typeAll("中文|", "**bold**"), "中文**bold**|");
  assertEquals(typeAll("中文，|", "**bold**"), "中文，**bold**|");
  assertEquals(typeAll("|", "*it*"), "*it*|");
  assertEquals(typeAll("|", "_it_"), "_it_|");
  assertEquals(typeAll("|", "`code`"), "`code`|");
});

Deno.test("a closing delimiter from the toolbar is stepped over", () => {
  assertEquals(typeAll("**bold|**", "*"), "**bold*|*");
  assertEquals(typeAll("**bold|**", "**"), "**bold**|");
  assertEquals(typeAll("~~x|~~", "~"), "~~x~|~~");
});

Deno.test("same-character tokens pair only between whitespace", () => {
  assertEquals(typeAll("中文|", "*"), "中文*|");
  assertEquals(typeAll("（|", "*"), "（*|");
  assertEquals(typeAll("(|)", "*"), "(*|)");
  assertEquals(typeAll("don|", "'"), "don'|");
  assertEquals(typeAll("say |", '"'), 'say "|"');
});

Deno.test("brackets pair before whitespace and closers, and step over", () => {
  assertEquals(typeAll("|", "("), "(|)");
  assertEquals(typeAll("|word", "("), "(|word");
  assertEquals(typeAll("|", "(a)"), "(a)|");
  assertEquals(typeAll("|", "[x]"), "[x]|");
});

Deno.test("selection wrapping follows Obsidian", () => {
  assertEquals(typeAll("<foo>", "*"), "*<foo>*");
  assertEquals(typeAll("<foo>", "("), "(<foo>)");
  assertEquals(typeAll("<foo>", "="), "=<foo>=");
  assertEquals(typeAll("<foo>", "=="), "==<foo>==");
  assertEquals(typeAll("<foo>", "~~"), "~~<foo>~~");
  assertEquals(typeAll("<a\nb>", "*"), "*|*");
  assertEquals(typeAll("<a\nb>", "`"), "`<a\nb>`");
  assertEquals(typeAll("a|", "="), "a=|");
});

Deno.test("three backticks open a fenced block with list indentation", () => {
  assertEquals(typeAll("|", "```"), "```|\n```");
  assertEquals(typeAll("- |", "```"), "- ```|\n  ```");
  assertEquals(typeAll("> |", "```"), "> ```|\n> ```");
  // A fence typed under a paragraph or list line still closes (lezer reads
  // that line as paragraph continuation, Obsidian's tokens do not).
  assertEquals(typeAll("Here is the log:\n|", "```"), "Here is the log:\n```|\n```");
  assertEquals(typeAll("- a\n  |", "```"), "- a\n  ```|\n  ```");
  assertEquals(typeAll("foo |", "```"), "foo ```|");
  // A manual closing fence inside an open block stays literal.
  assertEquals(typeAll("```\ncode\n|", "```"), "```\ncode\n```|");
});
