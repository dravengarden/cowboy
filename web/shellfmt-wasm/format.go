package main

import (
	"bytes"
	"path/filepath"
	"sort"
	"strings"

	"mvdan.cc/sh/v3/syntax"
)

type shellDisplay struct {
	Text    string
	Context string
}

func formatShellDisplay(source string) (shellDisplay, error) {
	file, err := parseShell(source)
	if err != nil {
		return shellDisplay{}, err
	}
	if inner, context, ok := wrappedShell(file); ok {
		text, err := formatParsedShell(inner)
		if err == nil {
			return shellDisplay{Text: text, Context: context}, nil
		}
	}
	text, err := formatParsedFile(file)
	return shellDisplay{Text: text}, err
}

func formatShellSource(source string) (string, error) {
	display, err := formatShellDisplay(source)
	return display.Text, err
}

func parseShell(source string) (*syntax.File, error) {
	return syntax.NewParser(syntax.Variant(syntax.LangBash)).Parse(bytes.NewBufferString(source), "")
}

func formatParsedShell(source string) (string, error) {
	file, err := parseShell(source)
	if err != nil {
		return "", err
	}
	return formatParsedFile(file)
}

func formatParsedFile(file *syntax.File) (string, error) {
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
	return insertStructuralBreaks(canonical, formatted), nil
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
