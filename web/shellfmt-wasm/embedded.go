package main

// Embedded-language discovery is deliberately separate from shell layout.
// mvdan/sh remains the authority for quoting, word boundaries, redirects, and
// heredocs; this registry only describes which static argv slot is source code.
// Adding a language must never require UI-specific parsing or command execution.

import (
	"path/filepath"
	"strings"
	"unicode/utf8"

	"mvdan.cc/sh/v3/syntax"
)

type payloadKind uint8

const (
	payloadFlag payloadKind = iota
	payloadFirstPositional
)

type embeddedLanguageSpec struct {
	commands     []string
	language     string
	label        string
	flags        []string
	kind         payloadKind
	minRunes     int
	structural   string
	minStructure int
	skipOptions  map[string]int
}

var embeddedLanguageSpecs = []embeddedLanguageSpec{
	{commands: []string{"python", "python3"}, language: "python", label: "Python", flags: []string{"-c"}, kind: payloadFlag, minRunes: 32, structural: "\n;:"},
	{commands: []string{"node", "nodejs"}, language: "javascript", label: "JavaScript", flags: []string{"-e", "--eval", "-p", "--print"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}=>"},
	{commands: []string{"deno"}, language: "typescript", label: "TypeScript", flags: []string{"eval"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}=>"},
	{commands: []string{"bun"}, language: "typescript", label: "TypeScript", flags: []string{"-e", "--eval"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}=>"},
	{commands: []string{"perl"}, language: "perl", label: "Perl", flags: []string{"-e", "-E"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}"},
	{commands: []string{"ruby"}, language: "ruby", label: "Ruby", flags: []string{"-e"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}"},
	{commands: []string{"php"}, language: "php", label: "PHP", flags: []string{"-r"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}"},
	{commands: []string{"lua", "luajit"}, language: "lua", label: "Lua", flags: []string{"-e"}, kind: payloadFlag, minRunes: 36, structural: "\n;{}"},
	{commands: []string{"awk", "gawk", "mawk"}, language: "awk", label: "AWK", kind: payloadFirstPositional, minRunes: 38, structural: "\n{};", skipOptions: map[string]int{"-F": 1, "-v": 1}},
	{commands: []string{"sed"}, language: "sed", label: "Sed", flags: []string{"-e", "--expression"}, kind: payloadFlag, minRunes: 42, structural: "\n;{}"},
	{commands: []string{"sed"}, language: "sed", label: "Sed", kind: payloadFirstPositional, minRunes: 42, structural: "\n;{}"},
	{commands: []string{"rg", "ripgrep"}, language: "regex", label: "Regex", flags: []string{"-e", "--regexp"}, kind: payloadFlag, minRunes: 52, structural: "|()[]{}?+*", minStructure: 6},
	{commands: []string{"rg", "ripgrep"}, language: "regex", label: "Regex", kind: payloadFirstPositional, minRunes: 52, structural: "|()[]{}?+*", minStructure: 6, skipOptions: map[string]int{
		"-A": 1, "--after-context": 1, "-B": 1, "--before-context": 1,
		"-C": 1, "--context": 1, "-f": 1, "--file": 1, "-g": 1,
		"--glob": 1, "--iglob": 1, "--ignore-file": 1, "-t": 1,
		"--type": 1, "-T": 1, "--type-not": 1, "--max-count": 1,
		"--max-depth": 1, "--max-filesize": 1, "--replace": 1,
	}},
	{commands: []string{"grep", "egrep"}, language: "regex", label: "Regex", flags: []string{"-e", "--regexp"}, kind: payloadFlag, minRunes: 52, structural: "|()[]{}?+*", minStructure: 6},
	{commands: []string{"grep", "egrep"}, language: "regex", label: "Regex", kind: payloadFirstPositional, minRunes: 52, structural: "|()[]{}?+*", minStructure: 6, skipOptions: map[string]int{
		"-A": 1, "--after-context": 1, "-B": 1, "--before-context": 1,
		"-C": 1, "--context": 1, "-f": 1, "--file": 1,
	}},
	{commands: []string{"pgrep", "pkill"}, language: "regex", label: "Regex", kind: payloadFirstPositional, minRunes: 52, structural: "|()[]{}?+*", minStructure: 6},
}

func embeddedClientPayload(args []*syntax.Word) (nestedPayload, bool) {
	if len(args) < 2 {
		return nestedPayload{}, false
	}
	command, ok := staticWord(args[0])
	if !ok {
		return nestedPayload{}, false
	}
	base := interpreterBase(command)
	// JSON is an argv payload rather than an executable-specific language. Use
	// the shell AST to decode quotes, accept only complete static JSON, and only
	// promote structures whose visual cost earns a separate frame. Small values
	// remain inline in Bash and are highlighted there.
	for _, word := range args[1:] {
		if payload, found := jsonPayload(word); found {
			return payload, true
		}
	}
	for _, spec := range embeddedLanguageSpecs {
		if !containsString(spec.commands, base) {
			continue
		}
		var payload nestedPayload
		var found bool
		if spec.kind == payloadFirstPositional {
			payload, found = firstPositionalPayload(args, spec)
		} else {
			payload, found = flaggedPayload(args, spec)
		}
		if found {
			return payload, true
		}
	}
	return nestedPayload{}, false
}

func jsonPayload(word *syntax.Word) (nestedPayload, bool) {
	value, static := staticWord(word)
	trimmed := strings.TrimSpace(value)
	if !static || len(trimmed) < 2 || (trimmed[0] != '{' && trimmed[0] != '[') {
		return nestedPayload{}, false
	}
	validShape, nodes, depth := jsonShape(trimmed)
	if !validShape || !jsonNeedsFrame(trimmed, nodes, depth) {
		return nestedPayload{}, false
	}
	return nestedPayload{
		start: int(word.Pos().Offset()), end: int(word.End().Offset()),
		launcher: "json", payload: trimmed, language: "json", label: "JSON",
	}, true
}

func jsonNeedsFrame(source string, nodes, depth int) bool {
	if strings.Contains(source, "\n") || utf8.RuneCountInString(source) >= 72 {
		return true
	}
	return nodes >= 7 || (depth >= 2 && nodes >= 4)
}

// jsonShape is deliberately a bounded structural recognizer, not a second JSON
// implementation. It rejects unbalanced strings/containers and counts only
// separators outside strings. The browser's native JSON.parse remains the
// authority before pretty-printing, so malformed payloads always fall back.
func jsonShape(source string) (valid bool, nodes, maxDepth int) {
	stack := make([]rune, 0, 8)
	inString, escaped := false, false
	nodes = 1
	for _, char := range source {
		if inString {
			if escaped {
				escaped = false
			} else if char == '\\' {
				escaped = true
			} else if char == '"' {
				inString = false
			}
			continue
		}
		switch char {
		case '"':
			inString = true
		case '{', '[':
			stack = append(stack, char)
			maxDepth = max(maxDepth, len(stack))
		case '}', ']':
			if len(stack) == 0 || (char == '}' && stack[len(stack)-1] != '{') || (char == ']' && stack[len(stack)-1] != '[') {
				return false, 0, 0
			}
			stack = stack[:len(stack)-1]
		case ',', ':':
			nodes++
		}
	}
	return !inString && !escaped && len(stack) == 0, nodes, maxDepth
}

func flaggedPayload(args []*syntax.Word, spec embeddedLanguageSpec) (nestedPayload, bool) {
	for index := 1; index < len(args); index++ {
		value, ok := staticWord(args[index])
		if !ok {
			return nestedPayload{}, false
		}
		if containsString(spec.flags, value) && index+1 < len(args) {
			return languagePayload(args[index+1], filepath.Base(mustStaticWord(args[0]))+" "+value, spec)
		}
		for _, flag := range spec.flags {
			if strings.HasPrefix(value, flag+"=") {
				// Inline flag values cannot be replaced without also removing the
				// executable option. Leave these readable as ordinary shell.
				return nestedPayload{}, false
			}
		}
	}
	return nestedPayload{}, false
}

func firstPositionalPayload(args []*syntax.Word, spec embeddedLanguageSpec) (nestedPayload, bool) {
	for index := 1; index < len(args); index++ {
		value, ok := staticWord(args[index])
		if !ok {
			return nestedPayload{}, false
		}
		if value == "--" {
			if index+1 < len(args) {
				return languagePayload(args[index+1], filepath.Base(mustStaticWord(args[0])), spec)
			}
			return nestedPayload{}, false
		}
		if consumed := spec.skipOptions[value]; consumed > 0 {
			index += consumed
			continue
		}
		if strings.HasPrefix(value, "-") && value != "-" {
			continue
		}
		return languagePayload(args[index], filepath.Base(mustStaticWord(args[0])), spec)
	}
	return nestedPayload{}, false
}

func languagePayload(word *syntax.Word, launcher string, spec embeddedLanguageSpec) (nestedPayload, bool) {
	value, ok := staticWord(word)
	if !ok || !embeddedNeedsFrame(value, spec) {
		return nestedPayload{}, false
	}
	return nestedPayload{
		start: int(word.Pos().Offset()), end: int(word.End().Offset()),
		launcher: launcher, payload: strings.TrimSpace(value),
		language: spec.language, label: spec.label,
	}, true
}

func embeddedNeedsFrame(source string, spec embeddedLanguageSpec) bool {
	trimmed := strings.TrimSpace(source)
	runes := utf8.RuneCountInString(trimmed)
	if runes >= spec.minRunes || strings.Contains(trimmed, "\n") {
		return true
	}
	count := 0
	for _, char := range trimmed {
		if strings.ContainsRune(spec.structural, char) {
			count++
		}
	}
	minimum := spec.minStructure
	if minimum == 0 {
		minimum = 3
	}
	// Dense punctuation is a useful secondary signal only when the payload also
	// occupies meaningful visual width. Short one-line expressions such as
	// `cowboy-v[0-9]+` remain inline (and syntax highlighted) instead of paying
	// the hierarchy cost of a separate frame.
	return runes >= min(spec.minRunes, 28) && count >= minimum
}

// embeddedHeredocPayload recognizes interpreter stdin without guessing from a
// delimiter name. The statement's executable identifies the language and the
// AST-provided Hdoc word supplies the exact decoded body.
func embeddedHeredocPayload(stmt *syntax.Stmt) (nestedPayload, bool) {
	call, ok := stmt.Cmd.(*syntax.CallExpr)
	if !ok || len(call.Args) == 0 {
		return nestedPayload{}, false
	}
	args := unwrappedClientArgs(call.Args)
	if len(args) == 0 {
		return nestedPayload{}, false
	}
	command, ok := staticWord(args[0])
	if !ok {
		return nestedPayload{}, false
	}
	base := interpreterBase(command)
	var language, label string
	switch {
	case base == "python" || strings.HasPrefix(base, "python3"):
		language, label = "python", "Python"
	case base == "node" || base == "nodejs":
		language, label = "javascript", "JavaScript"
	case base == "deno" || base == "bun":
		language, label = "typescript", "TypeScript"
	case base == "perl":
		language, label = "perl", "Perl"
	case base == "ruby":
		language, label = "ruby", "Ruby"
	case base == "php":
		language, label = "php", "PHP"
	case base == "lua" || base == "luajit":
		language, label = "lua", "Lua"
	default:
		return nestedPayload{}, false
	}
	for _, redirect := range stmt.Redirs {
		if (redirect.Op != syntax.Hdoc && redirect.Op != syntax.DashHdoc) || redirect.Hdoc == nil {
			continue
		}
		body, static := staticWord(redirect.Hdoc)
		if !static || strings.TrimSpace(body) == "" {
			return nestedPayload{}, false
		}
		terminator, static := staticWord(redirect.Word)
		if !static || terminator == "" {
			return nestedPayload{}, false
		}
		return nestedPayload{
			start: int(redirect.Hdoc.Pos().Offset()), end: int(redirect.Hdoc.End().Offset()),
			launcher: base + " stdin", payload: strings.TrimSpace(body), language: language, label: label,
			heredoc: true, terminator: terminator,
		}, true
	}
	return nestedPayload{}, false
}

func interpreterBase(command string) string {
	base := filepath.Base(command)
	if strings.HasPrefix(base, "python3.") {
		return "python3"
	}
	return base
}

func containsString(values []string, needle string) bool {
	for _, value := range values {
		if value == needle {
			return true
		}
	}
	return false
}

func mustStaticWord(word *syntax.Word) string {
	value, _ := staticWord(word)
	return value
}
