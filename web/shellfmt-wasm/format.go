package main

import (
	"bytes"
	"hash/fnv"
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
	Language string
	Label    string
	Dialect  string
	Depth    int
	Marker   string
	Color    int
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
	markers := newNestedShellMarkerAllocator(source)
	frames, err := formatShellFrames(source, file, columns, 0, &markers)
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
const maxShellFrames = 32
const nestedShellLongLineRunes = 72

var nestedShellMarkers = [...]string{
	"🔮", "🪐", "🌙", "⭐", "💎", "🧿", "🌀", "✨",
	"🚀", "🛸", "🛰️", "☄️", "🌌", "🌈", "🔥", "⚡",
	"🌊", "🍀", "🌸", "🍋", "🍊", "🍇", "🫐", "🥝",
	"🦊", "🐳", "🦋", "🐙", "🐝", "🐢", "🦄", "🐬",
	"🎯", "🎲", "🎮", "🕹️", "🎸", "🎹", "🎺", "🥁",
	"🧩", "🪄", "🎈", "🎨", "🧸", "🎁", "🎪", "🎭",
	"🍄", "🌵", "🌻", "🌺", "🪷", "🌴", "🌲", "🍁",
	"🐉", "🐈", "🦜", "🦩", "🐡", "🦀", "🐞", "🐧",
}

type nestedShellMarkerAllocator struct {
	next   int
	stride int
}

func newNestedShellMarkerAllocator(source string) nestedShellMarkerAllocator {
	hash := fnv.New64a()
	_, _ = hash.Write([]byte(source))
	sum := hash.Sum64()
	// The 64-entry palette and an odd stride are coprime, so every marker is
	// visited exactly once before repetition. The command-derived start and
	// stride make different trees feel varied while remaining stable across
	// renders, reloads, and devices.
	return nestedShellMarkerAllocator{
		next:   int(sum % uint64(len(nestedShellMarkers))),
		stride: int((sum>>8)%uint64(len(nestedShellMarkers)/2))*2 + 1,
	}
}

func (allocator *nestedShellMarkerAllocator) nextMarker() (string, int) {
	paletteIndex := allocator.next
	allocator.next = (allocator.next + allocator.stride) % len(nestedShellMarkers)
	return nestedShellMarkers[paletteIndex], paletteIndex
}

// formatShellFrames projects nested shell payloads as independently parsed
// source frames. The parent retains its execution skeleton with a compact
// colored payload reference only when its own direct syntax earns a frame.
// Transparent wrappers are removed after their descendants have been parsed,
// so complexity never bubbles upward from a child into an otherwise empty
// `bash -c` or SSH payload.
func formatShellFrames(source string, file *syntax.File, columns, depth int, markers *nestedShellMarkerAllocator) ([]shellFrame, error) {
	if depth < maxShellFrameDepth {
		nested := nestedPayloads(source, file)
		if len(nested) > 0 && len(nested) < maxShellFrames {
			type extraction struct {
				nested   nestedPayload
				children []shellFrame
			}
			extracted := make([]extraction, 0, len(nested))
			for _, candidate := range nested {
				marker, color := markers.nextMarker()
				var children []shellFrame
				var innerFile *syntax.File
				if candidate.language != "" {
					children = []shellFrame{{Text: candidate.payload, Language: candidate.language, Label: candidate.label, Dialect: candidate.dialect, Depth: depth + 1}}
				} else {
					var err error
					innerFile, err = parseShell(candidate.payload)
					if err != nil {
						continue
					}
					children, err = formatShellFrames(candidate.payload, innerFile, columns, depth+1, markers)
					if err != nil || len(children) == 0 {
						continue
					}
				}
				// A nested frame earns its extra visual hierarchy. Keep tiny leaf
				// payloads such as `true`, `echo ok`, or one short remote probe in
				// their parent command; extracting those costs more attention than
				// it saves. Structural shell complexity, multiple source lines, or a
				// genuinely long command justify a frame. A child which found deeper
				// frames must also remain so that the execution hierarchy is intact.
				if candidate.language == "" && len(children) == 1 && children[0].Launcher == "" && !nestedShellNeedsFrame(candidate.payload, innerFile) {
					continue
				}
				children[0].Launcher = candidate.launcher
				children[0].Marker = marker
				children[0].Color = color
				extracted = append(extracted, extraction{nested: candidate, children: children})
			}
			if len(extracted) > 0 {
				outer := source
				for index := len(extracted) - 1; index >= 0; index-- {
					child := extracted[index].nested
					marker := extracted[index].children[0].Marker
					// A quoted emoji is a valid, inert display payload. It preserves
					// the launcher's argument slot and pairs it with the matching
					// child rail without inventing prose or changing copied source.
					reference := "'" + marker + "'"
					if child.heredoc {
						// Hdoc.End includes both body and delimiter. Rebuild a tiny,
						// valid heredoc around the inert marker for the parent frame.
						reference += "\n" + child.terminator + "\n"
					}
					outer = outer[:child.start] + reference + outer[child.end:]
				}
				outerFile, err := parseShell(outer)
				if err == nil {
					outerText, err := formatParsedFile(outerFile, columns)
					if err == nil {
						frames := []shellFrame{{Text: outerText, Language: "bash", Label: "Bash", Depth: depth}}
						for _, child := range extracted {
							frames = append(frames, child.children...)
						}
						// Judge this node only after its immediate payloads have been
						// replaced by inert references. A complex grandchild must not
						// force a one-line wrapper into a separate visual frame. Promote
						// the descendants one level when that wrapper adds no structure.
						if depth > 0 && !directShellNeedsFrame(source, file) {
							frames = frames[1:]
							for index := range frames {
								frames[index].Depth--
							}
						}
						if len(frames) <= maxShellFrames {
							return frames, nil
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
	return []shellFrame{{Text: text, Language: "bash", Label: "Bash", Depth: depth}}, nil
}

// directShellNeedsFrame measures only syntax owned by this node. Every
// immediate embedded language or child shell is replaced with a harmless word
// before scoring, so a complex descendant cannot make a transparent launcher
// look complex. Descendants are evaluated independently by formatShellFrames.
func directShellNeedsFrame(source string, file *syntax.File) bool {
	nested := nestedPayloads(source, file)
	if len(nested) == 0 {
		return nestedShellNeedsFrame(source, file)
	}
	direct := source
	for index := len(nested) - 1; index >= 0; index-- {
		payload := nested[index]
		reference := "'x'"
		if payload.heredoc {
			reference += "\n" + payload.terminator + "\n"
		}
		direct = direct[:payload.start] + reference + direct[payload.end:]
	}
	directFile, err := parseShell(direct)
	if err != nil {
		// The original node parsed successfully. If its defensive projection
		// does not, retain the node instead of hiding potentially useful syntax.
		return true
	}
	return nestedShellNeedsFrame(direct, directFile)
}

// nestedShellNeedsFrame measures information density rather than matching
// command names. The fixed source-width threshold keeps the same command in
// the same mode on phone, tablet, and desktop; viewport columns only affect
// line layout after the hierarchy has been chosen.
func nestedShellNeedsFrame(source string, file *syntax.File) bool {
	statements := 0
	compound := false
	joins := 0
	syntax.Walk(file, func(node syntax.Node) bool {
		switch node.(type) {
		case *syntax.Stmt:
			statements++
		case *syntax.BinaryCmd:
			joins++
		case *syntax.IfClause, *syntax.ForClause, *syntax.WhileClause,
			*syntax.CaseClause, *syntax.FuncDecl:
			compound = true
		}
		return true
	})
	if compound || joins > 0 || statements >= 2 {
		return true
	}

	lines := strings.Split(strings.TrimSpace(source), "\n")
	if len(lines) >= 2 {
		return true
	}
	for _, line := range lines {
		if utf8.RuneCountInString(strings.TrimSpace(line)) >= nestedShellLongLineRunes {
			return true
		}
	}
	return false
}

type nestedPayload struct {
	start      int
	end        int
	launcher   string
	payload    string
	language   string
	label      string
	dialect    string
	heredoc    bool
	terminator string
}

func nestedPayloads(source string, file *syntax.File) []nestedPayload {
	found := make([]nestedPayload, 0, 4)
	syntax.Walk(file, func(node syntax.Node) bool {
		if statement, ok := node.(*syntax.Stmt); ok {
			if payload, foundPayload := embeddedHeredocPayload(statement); foundPayload {
				found = append(found, payload)
				// The heredoc word is part of this statement's tree. Once its exact
				// payload has been projected, do not discover a duplicate inside it.
				return false
			}
			return true
		}
		call, isCall := node.(*syntax.CallExpr)
		if !isCall || len(call.Args) < 2 {
			return true
		}
		// OpenSSH sends every argument after the destination to a remote shell.
		// Locally that script is only a Word (often assembled with the standard
		// `'”'”'` quote bridge), so it has no nested CallExpr until decoded.
		// Project the complete remote argv as one payload, then feed it back into
		// this same parser recursively. An explicit remote `bash -c` therefore
		// becomes another ordinary nested frame instead of a special UI case.
		if remote, ok := sshRemoteShell(source, call); ok {
			found = append(found, remote)
			return false
		}
		// Project-owned VM/device helpers commonly expose a remote shell as a
		// subcommand: `"$HELPER" ssh 'script'`. The executable is deliberately
		// dynamic, so it cannot be mistaken for OpenSSH and cannot be resolved by
		// shellInterpreterName. Treat the static `ssh` subcommand as the execution
		// boundary while preserving the dynamic launcher in the parent frame.
		if remote, ok := dynamicSSHSubcommand(source, call); ok {
			found = append(found, remote)
			return false
		}
		clientArgs := unwrappedClientArgs(call.Args)
		if sql, ok := sqlClientPayload(clientArgs); ok {
			found = append(found, sql)
			return false
		}
		if query, ok := jqClientPayload(clientArgs); ok {
			found = append(found, query)
			return false
		}
		if embedded, ok := embeddedClientPayload(clientArgs); ok {
			found = append(found, embedded)
			return false
		}
		for index := 0; index+2 < len(call.Args); index++ {
			shellWord, shellStatic := staticWord(call.Args[index])
			if !shellStatic {
				continue
			}
			shell, isShell := shellInterpreterName(shellWord)
			if !isShell {
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
			launcherArgs[index] = shell
			found = append(found, nestedPayload{
				start:    int(payload.Pos().Offset()),
				end:      int(payload.End().Offset()),
				launcher: strings.Join(launcherArgs, " "),
				payload:  payloadValue,
			})
			return false
		}
		return true
	})
	sort.Slice(found, func(i, j int) bool { return found[i].start < found[j].start })
	return found
}

var jqOptionsWithTwoValues = map[string]struct{}{
	"--arg": {}, "--argjson": {}, "--argfile": {}, "--rawfile": {}, "--slurpfile": {},
}

var jqOptionsWithOneValue = map[string]struct{}{
	"--indent": {}, "--library-path": {}, "-L": {},
}

var sudoOptionsWithOneValue = map[string]struct{}{
	"-C": {}, "--close-from": {}, "-D": {}, "--chdir": {}, "-g": {}, "--group": {},
	"-h": {}, "--host": {}, "-p": {}, "--prompt": {}, "-R": {}, "--chroot": {},
	"-T": {}, "--command-timeout": {}, "-u": {}, "--user": {},
}

// unwrappedClientArgs follows transparent command launchers without reparsing
// strings. Word positions still refer to the original shell source, so an
// extracted jq/SQL frame can replace only its payload while Source stays exact.
func unwrappedClientArgs(args []*syntax.Word) []*syntax.Word {
	for depth := 0; depth < 4 && len(args) > 1; depth++ {
		command, static := staticWord(args[0])
		if !static {
			return args
		}
		switch filepath.Base(command) {
		case "sudo":
			index := 1
			for index < len(args) {
				value, ok := staticWord(args[index])
				if !ok {
					return args
				}
				if value == "--" {
					index++
					break
				}
				if _, consumes := sudoOptionsWithOneValue[value]; consumes {
					index += 2
					continue
				}
				if strings.HasPrefix(value, "--user=") || strings.HasPrefix(value, "--group=") ||
					strings.HasPrefix(value, "--host=") || strings.HasPrefix(value, "--chdir=") ||
					strings.HasPrefix(value, "--chroot=") || strings.HasPrefix(value, "--prompt=") ||
					strings.HasPrefix(value, "--close-from=") || strings.HasPrefix(value, "--command-timeout=") {
					index++
					continue
				}
				if strings.HasPrefix(value, "-") && value != "-" {
					index++
					continue
				}
				if strings.Contains(value, "=") {
					index++
					continue
				}
				break
			}
			if index >= len(args) {
				return args
			}
			args = args[index:]
		case "command", "builtin", "nohup":
			index := 1
			for index < len(args) {
				value, ok := staticWord(args[index])
				if !ok || !strings.HasPrefix(value, "-") || value == "-" {
					break
				}
				index++
			}
			if index >= len(args) {
				return args
			}
			args = args[index:]
		case "env":
			index := 1
			for index < len(args) {
				value, ok := staticWord(args[index])
				if !ok {
					return args
				}
				if value == "--" {
					index++
					break
				}
				if value == "-u" || value == "--unset" || value == "-C" || value == "--chdir" {
					index += 2
					continue
				}
				if strings.HasPrefix(value, "-") || strings.Contains(value, "=") {
					index++
					continue
				}
				break
			}
			if index >= len(args) {
				return args
			}
			args = args[index:]
		case "timeout":
			index := 1
			for index < len(args) {
				value, ok := staticWord(args[index])
				if !ok {
					return args
				}
				if value == "-k" || value == "--kill-after" || value == "-s" || value == "--signal" {
					index += 2
					continue
				}
				if strings.HasPrefix(value, "-") {
					index++
					continue
				}
				// The first non-option is the duration; the next word is the
				// wrapped executable.
				index++
				break
			}
			if index >= len(args) {
				return args
			}
			args = args[index:]
		case "nix":
			if len(args) < 4 {
				return args
			}
			subcommand, ok := staticWord(args[1])
			if !ok || (subcommand != "develop" && subcommand != "shell") {
				return args
			}
			commandAt := -1
			for index := 2; index < len(args); index++ {
				value, ok := staticWord(args[index])
				if ok && (value == "-c" || value == "--command") {
					commandAt = index + 1
					break
				}
			}
			if commandAt < 0 || commandAt >= len(args) {
				return args
			}
			args = args[commandAt:]
		default:
			return args
		}
	}
	return args
}

// jqClientPayload extracts a static jq program from its shell argument. The
// shell AST remains the authority for quote and escape decoding; jqfmt then
// parses the decoded program before any display-only formatting is accepted.
// Small filters stay inline because a second visual frame would cost more
// attention than it saves.
func jqClientPayload(args []*syntax.Word) (nestedPayload, bool) {
	if len(args) < 2 {
		return nestedPayload{}, false
	}
	command, static := staticWord(args[0])
	if !static || filepath.Base(command) != "jq" {
		return nestedPayload{}, false
	}
	for index := 1; index < len(args); index++ {
		value, ok := staticWord(args[index])
		if !ok {
			return nestedPayload{}, false
		}
		if value == "--" {
			index++
			if index >= len(args) {
				return nestedPayload{}, false
			}
			return formattedJQPayload(args[index])
		}
		if value == "-f" || value == "--from-file" || strings.HasPrefix(value, "--from-file=") {
			return nestedPayload{}, false
		}
		if _, consumes := jqOptionsWithTwoValues[value]; consumes {
			index += 2
			if index >= len(args) {
				return nestedPayload{}, false
			}
			continue
		}
		if _, consumes := jqOptionsWithOneValue[value]; consumes {
			index++
			if index >= len(args) {
				return nestedPayload{}, false
			}
			continue
		}
		if strings.HasPrefix(value, "--arg=") || strings.HasPrefix(value, "--argjson=") ||
			strings.HasPrefix(value, "--indent=") || strings.HasPrefix(value, "--library-path=") {
			continue
		}
		if strings.HasPrefix(value, "-") && value != "-" {
			continue
		}
		return formattedJQPayload(args[index])
	}
	return nestedPayload{}, false
}

func formattedJQPayload(word *syntax.Word) (nestedPayload, bool) {
	source, static := staticWord(word)
	if !static || !jqNeedsFrame(source) {
		return nestedPayload{}, false
	}
	return nestedPayload{
		start: int(word.Pos().Offset()), end: int(word.End().Offset()),
		launcher: "jq", payload: strings.TrimSpace(source), language: "jq", label: "jq",
	}, true
}

// jqNeedsFrame is intentionally viewport-independent. Structural operators
// and nesting raise the score faster than raw length, while one short selector
// remains ordinary shell even on a phone.
func jqNeedsFrame(source string) bool {
	trimmed := strings.TrimSpace(source)
	if strings.Contains(trimmed, "\n") || utf8.RuneCountInString(trimmed) >= 64 {
		return true
	}
	depth, maxDepth, operators := 0, 0, 0
	inString, escaped := false, false
	for index := 0; index < len(trimmed); index++ {
		char := trimmed[index]
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
		case '(', '[', '{':
			depth++
			if depth > maxDepth {
				maxDepth = depth
			}
		case ')', ']', '}':
			if depth > 0 {
				depth--
			}
		case '|', ',':
			operators++
		}
	}
	return operators >= 2 || maxDepth >= 2
}

// sqlClientPayload extracts decoded SQL from PostgreSQL's client. staticWord
// is the authority for shell quote removal; dynamic payloads stay as Bash.
func sqlClientPayload(args []*syntax.Word) (nestedPayload, bool) {
	if len(args) < 3 {
		return nestedPayload{}, false
	}
	command, static := staticWord(args[0])
	if !static || filepath.Base(command) != "psql" {
		return nestedPayload{}, false
	}
	for index := 1; index+1 < len(args); index++ {
		option, ok := staticWord(args[index])
		if !ok || (option != "-c" && option != "--command") {
			continue
		}
		value, ok := staticWord(args[index+1])
		if !ok || strings.TrimSpace(value) == "" {
			return nestedPayload{}, false
		}
		// Keep tiny psql commands in the shell flow. A separate rail and marker
		// cost more attention than they save for meta commands such as `\d events`
		// or a short scalar query; complex SQL still earns syntax-aware framing.
		if !sqlNeedsFrame(value) {
			return nestedPayload{}, false
		}
		return nestedPayload{
			start: int(args[index+1].Pos().Offset()), end: int(args[index+1].End().Offset()),
			launcher: "psql " + option, payload: value, language: "sql", label: "SQL", dialect: "postgresql",
		}, true
	}
	return nestedPayload{}, false
}

// sqlNeedsFrame is viewport-independent so the same command keeps the same
// hierarchy across phone, tablet, and desktop. Newlines and long programs are
// inherently dense; otherwise multiple clauses, joins, or nested expressions
// provide enough structure for the dedicated SQL renderer to be worthwhile.
func sqlNeedsFrame(source string) bool {
	trimmed := strings.TrimSpace(source)
	if strings.Contains(trimmed, "\n") || utf8.RuneCountInString(trimmed) >= 64 {
		return true
	}
	upper := strings.ToUpper(trimmed)
	clauses := 0
	for _, keyword := range []string{
		"SELECT ", " FROM ", " WHERE ", " GROUP BY ", " ORDER BY ",
		" HAVING ", " LIMIT ", " JOIN ", " UNION ", " WITH ",
		" INSERT ", " UPDATE ", " DELETE ", " CREATE ", " ALTER ",
	} {
		if strings.Contains(" "+upper+" ", keyword) {
			clauses++
		}
	}
	return clauses >= 3 || strings.Count(trimmed, "(") >= 2 || strings.Count(trimmed, ";") >= 2
}

// shellInterpreterName recognizes real interpreter binary naming conventions,
// including Nix's versioned bash-interactive and wrapped executables, without
// treating every program whose name merely contains "bash" as a shell.
func shellInterpreterName(path string) (string, bool) {
	base := filepath.Base(path)
	for _, shell := range []string{"bash", "zsh"} {
		if base == shell || base == "."+shell+"-wrapped" || base == shell+"-wrapped" {
			return shell, true
		}
		version := strings.TrimPrefix(base, shell+"-")
		if version != base && version != "" {
			if shell == "bash" {
				version = strings.TrimPrefix(version, "interactive-")
			}
			if version != "" && version[0] >= '0' && version[0] <= '9' {
				return shell, true
			}
		}
	}
	return "sh", base == "sh" || base == ".sh-wrapped" || base == "sh-wrapped"
}

var sshOptionsWithValue = map[string]struct{}{
	"-B": {}, "-b": {}, "-c": {}, "-D": {}, "-E": {}, "-e": {},
	"-F": {}, "-I": {}, "-i": {}, "-J": {}, "-L": {}, "-l": {},
	"-m": {}, "-O": {}, "-o": {}, "-P": {}, "-p": {}, "-Q": {},
	"-R": {}, "-S": {}, "-W": {}, "-w": {},
}

func sshRemoteShell(source string, call *syntax.CallExpr) (nestedPayload, bool) {
	command, static := staticWord(call.Args[0])
	if !static || filepath.Base(command) != "ssh" {
		return nestedPayload{}, false
	}
	destination := -1
	for index := 1; index < len(call.Args); index++ {
		value, ok := staticWord(call.Args[index])
		if !ok {
			return nestedPayload{}, false
		}
		if value == "--" {
			if index+1 >= len(call.Args) {
				return nestedPayload{}, false
			}
			destination = index + 1
			break
		}
		if strings.HasPrefix(value, "-") && value != "-" {
			option := value
			if len(option) > 2 {
				option = option[:2]
			}
			if _, consumesValue := sshOptionsWithValue[option]; consumesValue && len(value) == 2 {
				index++
				if index >= len(call.Args) {
					return nestedPayload{}, false
				}
			}
			continue
		}
		destination = index
		break
	}
	remoteStart := destination + 1
	if destination < 0 || remoteStart >= len(call.Args) {
		return nestedPayload{}, false
	}
	remoteArgs := make([]string, 0, len(call.Args)-remoteStart)
	for _, word := range call.Args[remoteStart:] {
		value, ok := staticWord(word)
		if !ok {
			return nestedPayload{}, false
		}
		remoteArgs = append(remoteArgs, value)
	}
	payload := strings.Join(remoteArgs, " ")
	launcherArgs := make([]string, 0, remoteStart)
	for _, word := range call.Args[:remoteStart] {
		value, _ := staticWord(word)
		launcherArgs = append(launcherArgs, value)
	}
	launcherArgs[0] = filepath.Base(launcherArgs[0])
	remote := nestedPayload{
		start:    int(call.Args[remoteStart].Pos().Offset()),
		end:      int(call.Args[len(call.Args)-1].End().Offset()),
		launcher: strings.Join(launcherArgs, " "),
		payload:  payload,
	}
	if _, err := parseShell(payload); err != nil {
		if !opaqueRemoteShellNeedsFrame(payload) {
			return nestedPayload{}, false
		}
		remote.language, remote.label = remoteShellFallbackLanguage(payload)
	}
	return remote, true
}

// dynamicSSHSubcommand recognizes helper CLIs shaped like
// `"$HELPER" ssh 'remote shell source'`. Requiring a non-static launcher keeps
// this separate from ordinary programs which merely receive the word `ssh` as
// data. Every argument after the subcommand belongs to the remote shell, which
// matches the argv contract used by VM and device helpers.
func dynamicSSHSubcommand(source string, call *syntax.CallExpr) (nestedPayload, bool) {
	if len(call.Args) < 3 {
		return nestedPayload{}, false
	}
	if _, static := staticWord(call.Args[0]); static {
		return nestedPayload{}, false
	}
	subcommand, static := staticWord(call.Args[1])
	if !static || subcommand != "ssh" {
		return nestedPayload{}, false
	}
	remoteArgs := make([]string, 0, len(call.Args)-2)
	for _, word := range call.Args[2:] {
		value, ok := staticWord(word)
		if !ok {
			return nestedPayload{}, false
		}
		remoteArgs = append(remoteArgs, value)
	}
	payload := strings.Join(remoteArgs, " ")
	launcherStart := int(call.Args[0].Pos().Offset())
	launcherEnd := int(call.Args[1].End().Offset())
	remote := nestedPayload{
		start:    int(call.Args[2].Pos().Offset()),
		end:      int(call.Args[len(call.Args)-1].End().Offset()),
		launcher: source[launcherStart:launcherEnd],
		payload:  payload,
	}
	if _, err := parseShell(payload); err != nil {
		if !opaqueRemoteShellNeedsFrame(payload) {
			return nestedPayload{}, false
		}
		remote.language, remote.label = remoteShellFallbackLanguage(payload)
	}
	return remote, true
}

// A remote login shell is not necessarily Bash. macOS development hosts often
// run Fish, while appliances may expose another shell dialect. Keep complex,
// statically recovered payloads visible even when the Bash parser rejects them;
// syntax highlighting is best-effort and Source remains the exact authority.
func opaqueRemoteShellNeedsFrame(source string) bool {
	trimmed := strings.TrimSpace(source)
	return strings.Contains(trimmed, "\n") || utf8.RuneCountInString(trimmed) >= 72 ||
		strings.Count(trimmed, ";") >= 2 || strings.Count(trimmed, "|") >= 2
}

func remoteShellFallbackLanguage(source string) (string, string) {
	for _, signal := range []string{"; and ", "; or ", " end;", " end\n", "(seq ", "set -l ", "string match "} {
		if strings.Contains(source, signal) {
			return "fish", "Fish"
		}
	}
	return "shell", "Remote shell"
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
	readable := insertReadableBreaks(structured, structuredFile, columns)
	readableFile, err := parseShell(readable)
	if err != nil {
		return "", err
	}
	return separateTopLevelStatements(readable, readableFile), nil
}

// separateTopLevelStatements gives independent execution units one quiet line
// of breathing room. Pipelines, &&/|| chains, loops, conditions, and nested
// bodies remain internally contiguous because they belong to one top-level
// statement. This makes the visual grouping follow Bash semantics instead of
// command names or incidental wrapping.
func separateTopLevelStatements(source string, file *syntax.File) string {
	if len(file.Stmts) < 2 {
		return source
	}
	breaks := make([]int, 0, len(file.Stmts)-1)
	for index := 0; index+1 < len(file.Stmts); index++ {
		end := int(file.Stmts[index].End().Offset())
		next := int(file.Stmts[index+1].Pos().Offset())
		if end < 0 || next < end || next > len(source) {
			continue
		}
		if strings.Count(source[end:next], "\n") < 2 {
			breaks = append(breaks, end)
		}
	}
	for index := len(breaks) - 1; index >= 0; index-- {
		at := breaks[index]
		source = source[:at] + "\n" + source[at:]
	}
	return source
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
			value.WriteString(decodeUnquotedLiteral(part.Value))
		case *syntax.SglQuoted:
			value.WriteString(part.Value)
		case *syntax.DblQuoted:
			for _, nested := range part.Parts {
				literal, ok := nested.(*syntax.Lit)
				if !ok {
					return "", false
				}
				value.WriteString(decodeDoubleQuotedLiteral(literal.Value))
			}
		default:
			return "", false
		}
	}
	return value.String(), true
}

// mvdan preserves source backslashes in unquoted literals too. Outside quotes,
// Bash consumes a backslash before any following byte; a backslash-newline pair
// is a line continuation and contributes no byte. Decoding here turns standard
// quote bridges such as '\” back into the single quote an SSH or `bash -c`
// receiver actually sees, while the Source view continues to use exact offsets.
func decodeUnquotedLiteral(source string) string {
	var value strings.Builder
	for index := 0; index < len(source); index++ {
		if source[index] != '\\' || index+1 >= len(source) {
			value.WriteByte(source[index])
			continue
		}
		if source[index+1] != '\n' {
			value.WriteByte(source[index+1])
		}
		index++
	}
	return value.String()
}

// mvdan's syntax tree preserves source backslashes inside double quotes.
// Decode exactly the escapes Bash consumes there before recursively parsing a
// `bash -c` payload: dollar, backtick, quote, backslash, and line continuation.
// A backslash before any other byte remains literal per Bash semantics.
func decodeDoubleQuotedLiteral(source string) string {
	var value strings.Builder
	for index := 0; index < len(source); index++ {
		if source[index] != '\\' || index+1 >= len(source) {
			value.WriteByte(source[index])
			continue
		}
		next := source[index+1]
		switch next {
		case '$', '`', '"', '\\':
			value.WriteByte(next)
			index++
		case '\n':
			index++
		default:
			value.WriteByte('\\')
		}
	}
	return value.String()
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
