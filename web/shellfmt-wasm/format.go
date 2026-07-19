package main

import (
	"bytes"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	"mvdan.cc/sh/v3/syntax"
)

type shellDisplay struct {
	Text     string
	FlatText string
	Context  string
	Frames   []shellFrame
	Summary  string
}

type shellFrame struct {
	Launcher string
	Text     string
}

func formatShellDisplay(source string, columns int) (shellDisplay, error) {
	file, err := parseShell(source)
	if err != nil {
		return shellDisplay{}, err
	}
	flatText, err := formatParsedFile(file, columns)
	if err != nil {
		return shellDisplay{}, err
	}
	frames, err := formatShellFrames(source, file, columns, 0)
	if err != nil {
		return shellDisplay{}, err
	}
	display := shellDisplay{FlatText: flatText, Frames: frames}
	if len(frames) > 0 {
		display.Text = frames[len(frames)-1].Text
		for _, frame := range frames {
			if frame.Launcher != "" {
				display.Context = frame.Launcher
			}
		}
		display.Summary = summarizeFrames(frames)
	}
	return display, nil
}

const maxShellFrameDepth = 6

// formatShellFrames projects nested shell payloads as independently parsed
// source frames. The parent retains its execution skeleton without inventing a
// textual placeholder; the UI's colored nesting rail is the payload slot. Each
// launcher therefore appears once, in its parent frame, while the child gets
// Bash highlighting instead of being painted as one giant quoted string.
func formatShellFrames(source string, file *syntax.File, columns, depth int) ([]shellFrame, error) {
	if depth < maxShellFrameDepth {
		if nested, ok := firstNestedShell(source, file); ok {
			outer := source[:nested.start] + source[nested.end:]
			outerFile, err := parseShell(outer)
			if err == nil {
				outerText, err := formatParsedFile(outerFile, columns)
				if err == nil {
					innerFile, err := parseShell(nested.payload)
					if err == nil {
						children, err := formatShellFrames(nested.payload, innerFile, columns, depth+1)
						if err == nil {
							return append([]shellFrame{{Launcher: nested.launcher, Text: outerText}}, children...), nil
						}
					}
				}
			}
		}
	}
	text, err := formatParsedFile(file, columns)
	if err != nil {
		return nil, err
	}
	return []shellFrame{{Text: text}}, nil
}

type nestedShell struct {
	start    int
	end      int
	launcher string
	payload  string
}

func firstNestedShell(source string, file *syntax.File) (found nestedShell, ok bool) {
	syntax.Walk(file, func(node syntax.Node) bool {
		if ok {
			return false
		}
		call, isCall := node.(*syntax.CallExpr)
		if !isCall || len(call.Args) < 3 {
			return true
		}
		for index := 0; index+2 < len(call.Args); index++ {
			shellWord, shellStatic := staticWord(call.Args[index])
			if !shellStatic {
				continue
			}
			shell := filepath.Base(shellWord)
			if shell != "bash" && shell != "sh" && shell != "zsh" {
				continue
			}
			option, optionStatic := staticWord(call.Args[index+1])
			payloadValue, payloadStatic := staticWord(call.Args[index+2])
			if !optionStatic || !payloadStatic {
				continue
			}
			if !strings.HasPrefix(option, "-") || !strings.Contains(option[1:], "c") {
				continue
			}
			payload := call.Args[index+2]
			launcherArgs := make([]string, 0, index+2)
			for _, word := range call.Args[:index+2] {
				value, static := staticWord(word)
				if !static {
					value = source[int(word.Pos().Offset()):int(word.End().Offset())]
				}
				launcherArgs = append(launcherArgs, value)
			}
			// Nix store and toolchain wrappers commonly invoke Bash by an
			// immutable absolute path. The Source view retains that evidence;
			// the compact execution frame names the actual interpreter without
			// spending the phone width on a store hash.
			launcherArgs[index] = filepath.Base(launcherArgs[index])
			found = nestedShell{
				start:    int(payload.Pos().Offset()),
				end:      int(payload.End().Offset()),
				launcher: strings.Join(launcherArgs, " "),
				payload:  payloadValue,
			}
			ok = true
			return false
		}
		return true
	})
	return found, ok
}

func summarizeFrames(frames []shellFrame) string {
	parts := make([]string, 0, len(frames)+1)
	for _, frame := range frames {
		if frame.Launcher != "" {
			parts = append(parts, frame.Launcher)
		}
	}
	if len(frames) > 0 {
		for _, line := range strings.Split(frames[len(frames)-1].Text, "\n") {
			line = strings.TrimSpace(strings.TrimSuffix(line, "\\"))
			for _, operator := range []string{"&&", "||", "|"} {
				line = strings.TrimSpace(strings.TrimSuffix(line, operator))
			}
			if line != "" {
				parts = append(parts, line)
				break
			}
		}
	}
	return strings.Join(parts, " · ")
}

func formatShellSource(source string) (string, error) {
	display, err := formatShellDisplay(source, 80)
	return display.Text, err
}

func parseShell(source string) (*syntax.File, error) {
	return syntax.NewParser(syntax.Variant(syntax.LangBash)).Parse(bytes.NewBufferString(source), "")
}

func formatParsedShell(source string, columns int) (string, error) {
	file, err := parseShell(source)
	if err != nil {
		return "", err
	}
	return formatParsedFile(file, columns)
}

func formatParsedFile(file *syntax.File, columns int) (string, error) {
	var output bytes.Buffer
	printer := syntax.NewPrinter(
		syntax.Indent(2),
		syntax.BinaryNextLine(true),
		syntax.SwitchCaseIndent(true),
	)
	if err := printer.Print(&output, file); err != nil {
		return "", err
	}
	canonical := output.String()
	formatted, err := syntax.NewParser(syntax.Variant(syntax.LangBash)).Parse(strings.NewReader(canonical), "")
	if err != nil {
		return "", err
	}
	structured := insertStructuralBreaks(canonical, formatted)
	structuredFile, err := parseShell(structured)
	if err != nil {
		return "", err
	}
	return insertReadableBreaks(structured, structuredFile, columns), nil
}

// insertReadableBreaks lays out any long simple command by shell-word
// boundaries. The inserted backslash-newline is valid shell, so Readable stays
// faithful instead of looking like arbitrary prose wrapping. This is generic
// CallExpr handling: commands, flags and values need no command-specific rules.
func insertReadableBreaks(source string, file *syntax.File, columns int) string {
	if columns < 32 {
		columns = 32
	}
	breaks := make(map[int]string)
	syntax.Walk(file, func(node syntax.Node) bool {
		call, ok := node.(*syntax.CallExpr)
		if !ok {
			return true
		}
		words := make([]syntax.Node, 0, len(call.Assigns)+len(call.Args))
		for _, assign := range call.Assigns {
			words = append(words, assign)
		}
		for _, word := range call.Args {
			words = append(words, word)
		}
		if len(words) < 2 {
			return true
		}

		lineStart := strings.LastIndex(source[:int(words[0].Pos().Offset())], "\n") + 1
		indent := leadingIndent(source[lineStart:])
		virtualColumn := utf8.RuneCountInString(source[lineStart:int(words[0].End().Offset())])
		for index, word := range words[1:] {
			start, end := int(word.Pos().Offset()), int(word.End().Offset())
			between := source[int(words[0].End().Offset()):start]
			if newline := strings.LastIndex(between, "\n"); newline >= 0 {
				virtualColumn = utf8.RuneCountInString(between[newline+1:])
			}
			wordWidth := utf8.RuneCountInString(source[start:end])
			if virtualColumn+1+wordWidth > columns {
				breakStart := start
				continuationWidth := wordWidth
				// Keep the common `--flag value` unit together. We cannot know
				// every CLI schema, but shell syntax does tell us when the
				// preceding word is option-shaped and the current word is not.
				previous := words[index]
				previousText := source[int(previous.Pos().Offset()):int(previous.End().Offset())]
				currentText := source[start:end]
				if index > 0 && optionShaped(previousText) && !optionShaped(currentText) {
					breakStart = int(previous.Pos().Offset())
					continuationWidth += 1 + utf8.RuneCountInString(previousText)
				}
				space := breakStart
				for space > 0 && (source[space-1] == ' ' || source[space-1] == '\t') {
					space--
				}
				if space > 0 && space < start && source[space-1] != '\n' {
					breaks[space] = " \\\n" + indent + "  "
					virtualColumn = len(indent) + 2 + continuationWidth
				} else {
					virtualColumn += 1 + wordWidth
				}
			} else {
				virtualColumn += 1 + wordWidth
			}
			words[0] = word
		}
		return true
	})

	offsets := make([]int, 0, len(breaks))
	for offset := range breaks {
		offsets = append(offsets, offset)
	}
	sort.Sort(sort.Reverse(sort.IntSlice(offsets)))
	for _, offset := range offsets {
		end := offset
		for end < len(source) && (source[end] == ' ' || source[end] == '\t') {
			end++
		}
		source = source[:offset] + breaks[offset] + source[end:]
	}
	return source
}

func optionShaped(word string) bool {
	return strings.HasPrefix(word, "-") && word != "-" && !strings.Contains(word, "=")
}

func leadingIndent(line string) string {
	return line[:len(line)-len(strings.TrimLeft(line, " \t"))]
}

func staticWord(word *syntax.Word) (string, bool) {
	var value strings.Builder
	for _, part := range word.Parts {
		switch part := part.(type) {
		case *syntax.Lit:
			value.WriteString(part.Value)
		case *syntax.SglQuoted:
			value.WriteString(part.Value)
		case *syntax.DblQuoted:
			for _, nested := range part.Parts {
				literal, ok := nested.(*syntax.Lit)
				if !ok {
					return "", false
				}
				value.WriteString(literal.Value)
			}
		default:
			return "", false
		}
	}
	return value.String(), true
}

func insertStructuralBreaks(source string, file *syntax.File) string {
	type edit struct {
		start int
		end   int
		text  string
	}
	edits := make([]edit, 0, 12)
	indentAt := func(offset int) string {
		lineStart := strings.LastIndex(source[:offset], "\n") + 1
		return leadingIndent(source[lineStart:])
	}
	addEdit := func(next edit) {
		for _, existing := range edits {
			if existing.start == next.start && existing.end == next.end {
				return
			}
		}
		edits = append(edits, next)
	}
	breakAfter := func(offset, extraIndent int) {
		if offset <= 0 || offset > len(source) || source[offset-1] == '\n' {
			return
		}
		end := offset
		for end < len(source) && (source[end] == ' ' || source[end] == '\t') {
			end++
		}
		if end < len(source) && source[end] != '\n' {
			addEdit(edit{
				start: offset,
				end:   end,
				text:  "\n" + indentAt(offset) + strings.Repeat(" ", extraIndent),
			})
		}
	}
	breakBefore := func(offset int, indent string) {
		if offset <= 0 || offset > len(source) {
			return
		}
		start := offset
		for start > 0 && (source[start-1] == ' ' || source[start-1] == '\t') {
			start--
		}
		if start > 0 && source[start-1] == '\n' {
			return
		}
		// A semicolon immediately before a closing compound keyword is only a
		// grammar separator. Drop it when expanding the body; leaving `;\nfi`
		// is syntactically valid but visually looks like a detached statement.
		if start > 0 && source[start-1] == ';' {
			start--
		}
		addEdit(edit{start: start, end: offset, text: "\n" + indent})
	}
	syntax.Walk(file, func(node syntax.Node) bool {
		switch node := node.(type) {
		case *syntax.BinaryCmd:
			breakAfter(int(node.OpPos.Offset())+len(node.Op.String()), 2)
		case *syntax.Stmt:
			if node.Semicolon.IsValid() && !node.Background {
				offset := int(node.Semicolon.Offset()) + 1
				rest := strings.TrimLeft(source[offset:], " \t")
				// `; then`, `; do`, and closing keywords belong to their compound
				// syntax. Their owning AST node below decides whether to expand the
				// body; treating them as ordinary statement separators caused the
				// detached `then` shown on mobile.
				structural := false
				for _, keyword := range []string{"then", "do", "fi", "done", "else", "elif", "esac"} {
					if rest == keyword || strings.HasPrefix(rest, keyword+" ") || strings.HasPrefix(rest, keyword+"\n") {
						structural = true
						break
					}
				}
				if !structural {
					breakAfter(offset, 0)
				}
			}
		case *syntax.IfClause:
			if node.ThenPos.IsValid() {
				breakAfter(int(node.ThenPos.Offset())+len("then"), 2)
			}
			breakBefore(int(node.FiPos.Offset()), indentAt(int(node.Position.Offset())))
		case *syntax.WhileClause:
			breakAfter(int(node.DoPos.Offset())+len("do"), 2)
			breakBefore(int(node.DonePos.Offset()), indentAt(int(node.WhilePos.Offset())))
		case *syntax.ForClause:
			if !node.Braces {
				breakAfter(int(node.DoPos.Offset())+len("do"), 2)
				breakBefore(int(node.DonePos.Offset()), indentAt(int(node.ForPos.Offset())))
			}
		}
		return true
	})
	sort.Slice(edits, func(i, j int) bool { return edits[i].start > edits[j].start })
	for _, edit := range edits {
		source = source[:edit.start] + edit.text + source[edit.end:]
	}
	return source
}
