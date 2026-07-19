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
	Text    string
	Context string
}

func formatShellDisplay(source string, columns int) (shellDisplay, error) {
	file, err := parseShell(source)
	if err != nil {
		return shellDisplay{}, err
	}
	if inner, context, ok := wrappedShell(file); ok {
		text, err := formatParsedShell(inner, columns)
		if err == nil {
			return shellDisplay{Text: text, Context: context}, nil
		}
	}
	text, err := formatParsedFile(file, columns)
	return shellDisplay{Text: text}, err
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

func wrappedShell(file *syntax.File) (inner, context string, ok bool) {
	if len(file.Stmts) != 1 || len(file.Stmts[0].Redirs) > 0 {
		return "", "", false
	}
	call, ok := file.Stmts[0].Cmd.(*syntax.CallExpr)
	if !ok || len(call.Args) < 3 {
		return "", "", false
	}
	args := make([]string, len(call.Args))
	for index, word := range call.Args {
		value, static := staticWord(word)
		if !static {
			return "", "", false
		}
		args[index] = value
	}
	for index := 0; index+2 < len(args); index++ {
		shell := filepath.Base(args[index])
		if shell != "bash" && shell != "sh" && shell != "zsh" {
			continue
		}
		option := args[index+1]
		if !strings.HasPrefix(option, "-") || !strings.Contains(option[1:], "c") {
			continue
		}
		label := strings.ToUpper(shell[:1]) + shell[1:] + " " + option
		if index >= 2 && args[0] == "nix" && args[1] == "develop" {
			label = "Nix develop → " + label
		}
		return args[index+2], label, true
	}
	return "", "", false
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
	breaks := make([]int, 0, 8)
	syntax.Walk(file, func(node syntax.Node) bool {
		switch node := node.(type) {
		case *syntax.BinaryCmd:
			breaks = append(breaks, int(node.OpPos.Offset())+len(node.Op.String()))
		case *syntax.Stmt:
			if node.Semicolon.IsValid() && !node.Background {
				breaks = append(breaks, int(node.Semicolon.Offset())+1)
			}
		}
		return true
	})
	sort.Sort(sort.Reverse(sort.IntSlice(breaks)))
	for _, offset := range breaks {
		if offset <= 0 || offset > len(source) || source[offset-1] == '\n' {
			continue
		}
		end := offset
		for end < len(source) && (source[end] == ' ' || source[end] == '\t') {
			end++
		}
		if end < len(source) && source[end] == '\n' {
			continue
		}
		source = source[:offset] + "\n" + source[end:]
	}
	return source
}
