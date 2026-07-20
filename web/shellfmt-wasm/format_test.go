package main

import (
	"bufio"
	"encoding/base64"
	"os"
	"sort"
	"strconv"
	"strings"
	"testing"

	"mvdan.cc/sh/v3/syntax"
)

func TestFormatsShellStructure(t *testing.T) {
	formatted, err := formatShellSource("git status --short && nix flake update cowboy && git diff --check")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(formatted, "&&\n") {
		t.Fatalf("expected binary operators on readable line boundaries, got %q", formatted)
	}
}

func TestSeparatesIndependentTopLevelCommands(t *testing.T) {
	formatted, err := formatShellSource("git ls-files apps/device | head; git remote get-url origin")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(formatted, "head\n\ngit remote") {
		t.Fatalf("independent execution units need a quiet separator: %q", formatted)
	}
	if strings.Contains(formatted, "|\n\n") {
		t.Fatalf("a pipeline must remain one execution unit: %q", formatted)
	}
}

func TestKeepsCompoundBodiesVisuallyTogether(t *testing.T) {
	formatted, err := formatShellSource("for item in a b; do\n  prepare $item\n  verify $item\ndone\necho complete")
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(formatted, "prepare $item\n\n") || !strings.Contains(formatted, "done\n\necho complete") {
		t.Fatalf("only top-level execution units should be separated: %q", formatted)
	}
}

func TestRejectsInvalidShell(t *testing.T) {
	if _, err := formatShellSource("echo '"); err == nil {
		t.Fatal("expected an incomplete quote to fail closed")
	}
}

func TestNestedEmojiPaletteCoversFrameLimit(t *testing.T) {
	if len(nestedShellMarkers) < maxShellFrames {
		t.Fatalf("emoji palette must cover every allowed frame: markers=%d frames=%d", len(nestedShellMarkers), maxShellFrames)
	}
	seen := make(map[string]struct{}, len(nestedShellMarkers))
	for _, marker := range nestedShellMarkers {
		if _, duplicate := seen[marker]; duplicate {
			t.Fatalf("nested emoji markers must be unique: %q", marker)
		}
		seen[marker] = struct{}{}
	}
}

func TestNestedEmojiAllocationIsStableAndUnique(t *testing.T) {
	first := newNestedShellMarkerAllocator("nix develop -c bash -c 'cargo test'")
	again := newNestedShellMarkerAllocator("nix develop -c bash -c 'cargo test'")
	seen := make(map[string]struct{}, maxShellFrames)
	for range maxShellFrames {
		marker, color := first.nextMarker()
		againMarker, againColor := again.nextMarker()
		if marker != againMarker || color != againColor {
			t.Fatalf("the same command must retain stable markers: %q/%d != %q/%d", marker, color, againMarker, againColor)
		}
		if _, duplicate := seen[marker]; duplicate {
			t.Fatalf("a command tree must not repeat markers: %q", marker)
		}
		seen[marker] = struct{}{}
	}
}

func TestNestedEmojiDoesNotCollideWithPayloadText(t *testing.T) {
	allocator := newNestedShellMarkerAllocator("rg -n '⚡' web/src")
	for range maxShellFrames {
		marker, _ := allocator.nextMarker()
		if marker == "⚡" {
			t.Fatal("a source emoji must not be reused as a nested-frame reference")
		}
	}
}

// TestHistoricalShellCorpus is an opt-in, read-only production-corpus audit.
// The input is one base64-encoded command per line so multiline scripts remain
// distinct. Normal unit tests skip it; operators can export a corpus without
// checking commands or potentially sensitive arguments into Git.
func TestHistoricalShellCorpus(t *testing.T) {
	path := os.Getenv("COWBOY_SHELL_CORPUS")
	if path == "" {
		t.Skip("set COWBOY_SHELL_CORPUS to audit historical commands")
	}
	file, err := os.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	total, formatted, fallback, nested, ssh, heredoc, multiline := 0, 0, 0, 0, 0, 0, 0
	sshNested, sshFallback, shellCommand, shellCommandNested := 0, 0, 0, 0
	eligibleFrames, projectedFrames, missedFrames := 0, 0, 0
	maxFrames := 0
	errorCounts := make(map[string]int)
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 64*1024), 2*1024*1024)
	for scanner.Scan() {
		decoded, err := base64.StdEncoding.DecodeString(scanner.Text())
		if err != nil {
			t.Fatal(err)
		}
		command := string(decoded)
		total++
		hasSSH := strings.Contains(command, "ssh ") || strings.HasPrefix(command, "ssh ")
		if hasSSH {
			ssh++
		}
		hasShellCommand := strings.Contains(command, "bash -c") || strings.Contains(command, "bash -lc") || strings.Contains(command, "sh -c") || strings.Contains(command, "sh -lc")
		if hasShellCommand {
			shellCommand++
		}
		if strings.Contains(command, "<<") {
			heredoc++
		}
		if strings.Contains(command, "\n") {
			multiline++
		}
		display, err := formatShellDisplay(command, 46)
		if err != nil {
			fallback++
			if hasSSH {
				sshFallback++
			}
			errorCounts[err.Error()]++
			continue
		}
		formatted++
		if parsed, parseErr := parseShell(command); parseErr == nil {
			required := requiredNestedLaunchers(command, parsed, 0)
			eligibleFrames += len(required)
			actual := max(0, len(display.Frames)-1)
			projectedFrames += min(len(required), actual)
			missedFrames += max(0, len(required)-actual)
		}
		if len(display.Frames) > 1 {
			nested++
			if hasSSH {
				sshNested++
			}
			if hasShellCommand {
				shellCommandNested++
			}
		}
		maxFrames = max(maxFrames, len(display.Frames))
	}
	if err := scanner.Err(); err != nil {
		t.Fatal(err)
	}
	t.Logf("commands=%d formatted=%d fallback=%d nested=%d ssh=%d heredoc=%d multiline=%d max_frames=%d", total, formatted, fallback, nested, ssh, heredoc, multiline, maxFrames)
	t.Logf("ssh_nested=%d ssh_fallback=%d ssh_plain=%d shell_c=%d shell_c_nested=%d", sshNested, sshFallback, ssh-sshNested-sshFallback, shellCommand, shellCommandNested)
	t.Logf("eligible_nested_frames=%d projected_nested_frames=%d missed_nested_frames=%d", eligibleFrames, projectedFrames, missedFrames)
	if missedFrames != 0 {
		t.Errorf("historical corpus has %d eligible nested frames which were not projected", missedFrames)
	}
	errors := make([]string, 0, len(errorCounts))
	for message, count := range errorCounts {
		errors = append(errors, message+" ["+strconv.Itoa(count)+"]")
	}
	sort.Strings(errors)
	for _, summary := range errors {
		t.Log("fallback:", summary)
	}
}

func requiredNestedLaunchers(source string, parsed *syntax.File, depth int) []string {
	if depth >= maxShellFrameDepth {
		return nil
	}
	var launchers []string
	for _, candidate := range nestedPayloads(source, parsed) {
		if candidate.language != "" {
			launchers = append(launchers, candidate.launcher)
			continue
		}
		inner, err := parseShell(candidate.payload)
		if err != nil {
			continue
		}
		children := requiredNestedLaunchers(candidate.payload, inner, depth+1)
		if directShellNeedsFrame(candidate.payload, inner) {
			launchers = append(launchers, candidate.launcher)
		}
		launchers = append(launchers, children...)
	}
	return launchers
}

func TestExtractsNixBashCommandPayload(t *testing.T) {
	display, err := formatShellDisplay("nix develop -c bash -lc 'cargo fmt --check && cargo test'", 80)
	if err != nil {
		t.Fatal(err)
	}
	if display.Context != "nix develop -c bash -lc" {
		t.Fatalf("unexpected context %q", display.Context)
	}
	if strings.Contains(display.Text, "nix develop") || !strings.Contains(display.Text, "&&\n") {
		t.Fatalf("expected readable inner script, got %q", display.Text)
	}
}

func TestProjectsNestedShellsAsSourceFrames(t *testing.T) {
	display, err := formatShellDisplay(`set -e; nix develop -c bash -lc 'ssh host sh -c "printf ok && cargo test"'; echo done`, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("expected the transparent transport wrapper to collapse, got %#v", display.Frames)
	}
	if display.Frames[1].Launcher != "nix develop -c bash -lc" {
		t.Fatalf("unexpected launchers %#v", display.Frames)
	}
	if display.Frames[0].Depth != 0 || display.Frames[1].Depth != 1 {
		t.Fatalf("expected a real parent/child depth chain: %#v", display.Frames)
	}
	if strings.Contains(display.Frames[0].Text, "…") || !strings.Contains(display.Frames[0].Text, "nix develop -c bash -lc") {
		t.Fatalf("outer script must use its execution skeleton without a textual placeholder: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, "sh -c") || !strings.Contains(display.Frames[1].Text, "printf") {
		t.Fatalf("deepest source and compact summary missing: %#v", display)
	}
}

func TestWholeFileWrapperUsesTheSameParentFrameShape(t *testing.T) {
	display, err := formatShellDisplay("nix develop -c bash -c 'cd web && deno task test'", 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 || display.Frames[1].Launcher == "" {
		t.Fatalf("whole-file wrapper should be a parent skeleton followed by its payload: %#v", display.Frames)
	}
	expectedParent := "nix develop -c bash -c '" + display.Frames[1].Marker + "'"
	if strings.TrimSpace(display.Frames[0].Text) != expectedParent {
		t.Fatalf("parent skeleton should reference its stable child marker: %#v", display.Frames)
	}
	if strings.Contains(display.Frames[1].Text, "nix develop") {
		t.Fatalf("child frame must not repeat its launcher: %#v", display.Frames)
	}
	if !strings.Contains(display.FlatText, "nix develop") || !strings.Contains(display.FlatText, "deno task test") {
		t.Fatalf("ordinary readable mode must retain the complete command: %q", display.FlatText)
	}
}

func TestProjectsEverySiblingNestedShell(t *testing.T) {
	display, err := formatShellDisplay(`git diff --check && nix develop -c bash -c 'cd web && deno task check' && nix develop -c bash -c 'cd api && cargo test'`, 54)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 3 {
		t.Fatalf("expected one parent and both sibling payloads, got %#v", display.Frames)
	}
	if display.Frames[1].Depth != 1 || display.Frames[2].Depth != 1 {
		t.Fatalf("sibling payloads must retain the same depth: %#v", display.Frames)
	}
	if display.Frames[1].Marker == "" || display.Frames[2].Marker == "" || display.Frames[1].Marker == display.Frames[2].Marker {
		t.Fatalf("sibling payloads need distinct paired markers: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[0].Text, display.Frames[1].Marker) || !strings.Contains(display.Frames[0].Text, display.Frames[2].Marker) {
		t.Fatalf("parent payload slots must show matching markers: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, "deno task check") || !strings.Contains(display.Frames[2].Text, "cargo test") {
		t.Fatalf("both sibling payloads must remain visible: %#v", display.Frames)
	}
	if strings.Contains(display.Frames[0].Text, "deno task") || strings.Contains(display.Frames[0].Text, "cargo test") {
		t.Fatalf("parent skeleton must not duplicate extracted payloads: %#v", display.Frames)
	}
}

func TestSeparatesIndependentCommandsInsideNestedFrames(t *testing.T) {
	display, err := formatShellDisplay(`/nix/store/hash-bash-interactive-5.3p9/bin/bash-interactive-5.3p9 -lc "sed -n '120,270p' manager.go; sed -n '110,200p' overlay.go"`, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("expected an interpreter skeleton and one nested payload: %#v", display.Frames)
	}
	child := display.Frames[1].Text
	if !strings.Contains(child, "manager.go\n\nsed") {
		t.Fatalf("independent commands inside a nested payload need the same quiet separator: %q", child)
	}
}

func TestNestedGroupingPreservesExecutionUnits(t *testing.T) {
	source := `bash -lc 'prepare | validate
if ready; then
  deploy && verify
  report
fi
cleanup'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("expected one nested payload: %#v", display.Frames)
	}
	child := display.Frames[1].Text
	if strings.Contains(child, "|\n\n") || strings.Contains(child, "deploy &&\n\n") || strings.Contains(child, "verify\n\n  report") {
		t.Fatalf("pipelines, boolean chains, and compound bodies must stay visually contiguous: %q", child)
	}
	if !strings.Contains(child, "fi\n\ncleanup") {
		t.Fatalf("the next independent top-level command should be separated: %q", child)
	}
}

func TestNestedMarkersRemainUniqueAcrossDepths(t *testing.T) {
	display, err := formatShellDisplay(`bash -c 'printf outer; bash -c "printf inner; printf detail"'`, 54)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 3 {
		t.Fatalf("expected root, child, and grandchild frames: %#v", display.Frames)
	}
	if display.Frames[1].Depth != 1 || display.Frames[1].Marker == "" ||
		display.Frames[2].Depth != 2 || display.Frames[2].Marker == "" ||
		display.Frames[1].Marker == display.Frames[2].Marker {
		t.Fatalf("nested references must remain unique across the tree: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[0].Text, display.Frames[1].Marker) || !strings.Contains(display.Frames[1].Text, display.Frames[2].Marker) {
		t.Fatalf("each parent must contain its direct child's matching reference: %#v", display.Frames)
	}
}

func TestProjectsQuotedSSHRemoteScript(t *testing.T) {
	source := `ssh -o BatchMode=yes macbook-air 'bash -lc '"'"'printf ok | tail -1; test -f "/tmp/a b" && cat "/tmp/a b" || true'"'"''`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("expected the transparent remote bash wrapper to collapse: %#v", display.Frames)
	}
	if display.Frames[1].Launcher != "ssh -o BatchMode=yes macbook-air" {
		t.Fatalf("unexpected ssh nesting launchers: %#v", display.Frames)
	}
	if display.Frames[1].Depth != 1 || display.Frames[1].Marker == "" {
		t.Fatalf("the promoted remote script must retain the ssh reference: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, `test -f "/tmp/a b"`) {
		t.Fatalf("decoded remote script was not preserved: %#v", display.Frames)
	}
}

func TestNestedComplexityKeepsParentWithDirectWork(t *testing.T) {
	source := `ssh host 'echo before; bash -c "printf one; printf two"; echo after'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 3 {
		t.Fatalf("a parent with its own direct work must remain visible: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, "echo before") || !strings.Contains(display.Frames[1].Text, "echo after") {
		t.Fatalf("direct parent work was not preserved: %#v", display.Frames)
	}
	if display.Frames[2].Depth != 2 || !strings.Contains(display.Frames[2].Text, "printf one") {
		t.Fatalf("complex child must remain nested beneath its meaningful parent: %#v", display.Frames)
	}
}

func TestProjectsDynamicHelperSSHSubcommand(t *testing.T) {
	source := `VM=/opt/tools/vm.sh
VM_DIR=/tmp/falcon VM_PORT=2247 "$VM" ssh 'systemctl is-active force-ipv4-egress.service; systemctl list-jobs --no-pager; ip -6 rule show'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("expected helper transport and remote shell frames, got %#v", display.Frames)
	}
	if display.Frames[1].Launcher != `"$VM" ssh` || display.Frames[1].Depth != 1 {
		t.Fatalf("unexpected helper launcher frame: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, "systemctl list-jobs") || !strings.Contains(display.Frames[1].Text, "\n\nip -6 rule show") {
		t.Fatalf("remote commands should be readable and independently grouped: %q", display.Frames[1].Text)
	}
	if strings.Contains(display.Frames[0].Text, "systemctl is-active") || !strings.Contains(display.Frames[0].Text, display.Frames[1].Marker) {
		t.Fatalf("parent helper call should reference, not duplicate, its child: %#v", display.Frames)
	}
}

func TestPreservesLauncherCase(t *testing.T) {
	display, err := formatShellDisplay("BASH -lc 'echo ok'", 80)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 1 || display.Context != "" {
		t.Fatalf("shell matching remains case-sensitive like execution: %#v", display)
	}
}

func TestCompactsAbsoluteShellLauncherWithoutChangingSource(t *testing.T) {
	display, err := formatShellDisplay("/nix/store/hash-bash/bin/bash -lc 'sed -n 1,2p file && grep ok file'", 80)
	if err != nil {
		t.Fatal(err)
	}
	if display.Context != "bash -lc" || !strings.HasPrefix(display.Summary, "bash -lc · sed") {
		t.Fatalf("expected a compact interpreter frame, got %#v", display)
	}
}

func TestRecognizesVersionedNixShellInterpreters(t *testing.T) {
	source := `/nix/store/hash-bash-interactive-5.3p9/bin/bash-interactive-5.3p9 -lc "sed -n '1,190p' web/shellfmt-wasm/format_test.go; rg -n 'Frames|Context|Marker' web/shellfmt-wasm/format_test.go"`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 || display.Frames[1].Launcher != "bash -lc" {
		t.Fatalf("expected the Nix interpreter to expose its complex payload: %#v", display.Frames)
	}
}

func TestNestedRegexRemainsAGrandchildOfBash(t *testing.T) {
	source := `/nix/store/hash-bash-interactive-5.3p9/bin/bash-interactive-5.3p9 -lc "sed -n '240,455p' web/src/tools/blocks.tsx && rg -n 'Shell|Regex|JQ|SQL|embeddedNeedsFrame|cowboy-v\\[0-9\\]' web/src"`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 3 {
		t.Fatalf("expected root, Bash payload, and Regex payload: %#v", display.Frames)
	}
	if display.Frames[1].Language != "bash" || display.Frames[1].Depth != 1 ||
		display.Frames[2].Language != "regex" || display.Frames[2].Depth != 2 {
		t.Fatalf("Regex must remain enclosed by its Bash parent: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[0].Text, display.Frames[1].Marker) ||
		!strings.Contains(display.Frames[1].Text, display.Frames[2].Marker) {
		t.Fatalf("each marker must appear in its direct parent: %#v", display.Frames)
	}
}

func TestShellInterpreterNamesRejectLookalikes(t *testing.T) {
	for _, path := range []string{"BASH", "bash-language-server", "sh-syntax", "mybash"} {
		if name, ok := shellInterpreterName(path); ok {
			t.Fatalf("lookalike %q must not be recognized as %q", path, name)
		}
	}
}

func TestFindsNestedPayloadWithDynamicTrailingArguments(t *testing.T) {
	display, err := formatShellDisplay(`nix develop -c env TOKEN="$TOKEN" bash -c 'prepare "$1" && exec worker "$1"' _ "$root"`, 80)
	if err != nil {
		t.Fatal(err)
	}
	if display.Context != `nix develop -c env TOKEN="$TOKEN" bash -c` || !strings.Contains(display.Text, `exec worker "$1"`) {
		t.Fatalf("expected nested payload despite dynamic argv, got %#v", display)
	}
}

func TestKeepsTrivialNestedPayloadsInline(t *testing.T) {
	for _, source := range []string{
		`bash -lc 'true'`,
		`bash -lc 'echo ok'`,
		`ssh host 'sudo systemctl reboot'`,
	} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatalf("%q: %v", source, err)
		}
		if len(display.Frames) != 1 {
			t.Fatalf("trivial payload should stay inline for %q: %#v", source, display.Frames)
		}
	}
}

func TestExtractsJQThroughTransparentCommandWrappers(t *testing.T) {
	for _, source := range []string{
		`sudo jq '{revision,profile:{outbounds:[.profile.outbounds[]? | select(.type=="tailscale") | {type,tag,server_url}]}}' state.json`,
		`sudo -u operator -- jq -r '.items[] | select(.ready) | [.id,.name] | @tsv' state.json`,
		`command jq '.items[] | select(.ready) | {id,name}' state.json`,
	} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatalf("%q: %v", source, err)
		}
		if len(display.Frames) != 2 || display.Frames[1].Language != "jq" {
			t.Fatalf("transparent wrapper should expose jq payload for %q: %#v", source, display.Frames)
		}
	}
}

func TestHistoricalQuotedNestedCommands(t *testing.T) {
	cases := []string{
		`/nix/store/0641h8qfqaxnwrsw2nzrz6i1wbzyx92l-bash-interactive-5.3p9/bin/bash -lc "sed -n '1710,1875p' web/src/Transcript.tsx && rg -n \"const run|itemIndex|runTitle|function ToolDetailsBrowser|select =\" web/src/Transcript.tsx web/src/tools/presentation.ts* web/src/**/*.test.*"`,
		`nix develop -c bash -c "cd web && deno eval 'console.log(\".endpoints[] | select(.type == tailscale)\")'"`,
	}
	for _, source := range cases {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatalf("%q: %v", source, err)
		}
		if len(display.Frames) < 2 {
			t.Fatalf("historical nested command was not extracted: %#v", display)
		}
	}
}

func TestDecodesBashDoubleQuotedEscapesForNestedParsing(t *testing.T) {
	got := decodeDoubleQuotedLiteral("quoted=\\\"yes\\\" dollar=\\$HOME slash=\\\\ keep=\\q joined=one\\\ntwo")
	want := "quoted=\"yes\" dollar=$HOME slash=\\ keep=\\q joined=onetwo"
	if got != want {
		t.Fatalf("decoded double-quoted literal = %q, want %q", got, want)
	}
}

func TestExtractsNestedPayloadByGeneralComplexity(t *testing.T) {
	longArgument := strings.Repeat("segment/", 10)
	for _, source := range []string{
		`bash -lc 'prepare && verify'`,
		"bash -lc 'first\nsecond'",
		`bash -lc 'if ready; then deploy; fi'`,
		`bash -lc 'cat /` + longArgument + `artifact'`,
	} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatalf("%q: %v", source, err)
		}
		if len(display.Frames) < 2 {
			t.Fatalf("complex payload should earn a nested frame for %q: %#v", source, display.Frames)
		}
	}
}

func TestWrapsLongCallsAtShellWordBoundaries(t *testing.T) {
	display, err := formatShellDisplay("psql 'postgresql://cowboy?host=/run/postgresql-cowboy&port=5433&user=cowboy' -P pager=off -F $'\\t' -A -c 'select id, title from sessions order by updated_at desc'", 46)
	if err != nil {
		t.Fatal(err)
	}
	formatted := display.Frames[0].Text
	if !strings.Contains(formatted, " \\\n  -P pager=off") {
		t.Fatalf("expected a shell continuation before arguments, got %q", formatted)
	}
	if _, err := parseShell(formatted); err != nil {
		t.Fatalf("readable layout must remain valid shell: %v", err)
	}
}

func TestKeepsAssignmentValueAtomic(t *testing.T) {
	formatted, err := formatShellSource("BRIDGE=/home/draven/columbus/machines/hawk/nixos/.agents/skills/chrome-debug-bridge/helpers/bridge.sh; $BRIDGE up")
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(formatted, "BRIDGE= \\") {
		t.Fatalf("assignment value must not be separated from its name: %q", formatted)
	}
}

func TestExpandsInlineIfBodyWithoutDetachingThen(t *testing.T) {
	formatted, err := formatShellSource(`for attempt in 1 2; do
  if test "$revision" = 16 && test "$state" = connected; then exit 0; fi
done`)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(formatted, ";\n  then") || strings.Contains(formatted, ";\nthen") {
		t.Fatalf("then must remain attached to its condition, got %q", formatted)
	}
	if !strings.Contains(formatted, "then\n    exit 0\n  fi") {
		t.Fatalf("expected an indented multiline if body, got %q", formatted)
	}
}

func TestExtractsPsqlCommandAsPostgresqlFrame(t *testing.T) {
	source := `psql "postgresql://cowboy?host=/run/postgresql-cowboy&port=5433" -X -At -c "select replace(encode(convert_to(payload->>'command', 'UTF8'), 'base64'), chr(10), '') from events where payload->>'sessionUpdate'='tool_call'" | go test ./web/shellfmt-wasm`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	var sql *shellFrame
	for index := range display.Frames {
		if display.Frames[index].Language == "sql" {
			sql = &display.Frames[index]
			break
		}
	}
	if sql == nil {
		t.Fatalf("expected a nested SQL frame: %#v", display.Frames)
	}
	if sql.Dialect != "postgresql" || sql.Launcher != "psql -c" {
		t.Fatalf("expected PostgreSQL metadata: %#v", sql)
	}
	if strings.Contains(sql.Text, `\'`) || !strings.Contains(sql.Text, "payload->>'command'") {
		t.Fatalf("SQL must be decoded from Bash quoting before formatting: %q", sql.Text)
	}
}

func TestLeavesDynamicPsqlPayloadInShellFrame(t *testing.T) {
	display, err := formatShellDisplay(`psql database -c "$QUERY"`, 46)
	if err != nil {
		t.Fatal(err)
	}
	for _, frame := range display.Frames {
		if frame.Language == "sql" {
			t.Fatalf("dynamic SQL cannot be safely extracted: %#v", display.Frames)
		}
	}
}

func TestKeepsSimplePsqlPayloadsInline(t *testing.T) {
	for _, source := range []string{
		`psql database -c '\d events'`,
		`psql database --command 'select 1'`,
	} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatal(err)
		}
		for _, frame := range display.Frames {
			if frame.Language == "sql" {
				t.Fatalf("simple SQL must stay inline: source=%q frames=%#v", source, display.Frames)
			}
		}
	}
}

func TestExtractsComplexJQProgramAsFormattedFrame(t *testing.T) {
	source := `jq -er '.endpoints[] | select(.type == "tailscale" and (has("detour") | not) and .domain_resolver.server == "dns-direct") | .inbounds[] | select(.type == "tun") | .stack' "$tmp"`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	var query *shellFrame
	for index := range display.Frames {
		if display.Frames[index].Language == "jq" {
			query = &display.Frames[index]
			break
		}
	}
	if query == nil {
		t.Fatalf("expected a nested jq frame: %#v", display.Frames)
	}
	if query.Launcher != "jq" {
		t.Fatalf("expected jq metadata: %#v", query)
	}
	if strings.Contains(query.Text, `\"`) || !strings.Contains(query.Text, `"tailscale"`) {
		t.Fatalf("jq must be decoded from Bash quoting before formatting: %q", query.Text)
	}
}

func TestExtractsJQProgramAfterArgumentOptions(t *testing.T) {
	display, err := formatShellDisplay(`jq --arg kind tailscale --argjson enabled true '.items[] | select(.type == $kind and .enabled == $enabled) | {id, type, enabled}' input.json`, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) < 2 || display.Frames[1].Language != "jq" {
		t.Fatalf("expected option values to be skipped before the jq program: %#v", display.Frames)
	}
}

func TestKeepsSimpleAndDynamicJQProgramsInline(t *testing.T) {
	for _, source := range []string{`jq -r '.remote_url' profile.json`, `jq "$FILTER" profile.json`, `jq -f filter.jq profile.json`} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatal(err)
		}
		for _, frame := range display.Frames {
			if frame.Language == "jq" {
				t.Fatalf("simple, dynamic, or file-backed jq must stay inline: source=%q frames=%#v", source, display.Frames)
			}
		}
	}
}

func TestLeavesInvalidJQProgramInShellFrame(t *testing.T) {
	display, err := formatShellDisplay(`jq '.items[] | select(' input.json`, 46)
	if err != nil {
		t.Fatal(err)
	}
	for _, frame := range display.Frames {
		if frame.Language == "jq" {
			t.Fatalf("invalid jq must fail closed to Bash: %#v", display.Frames)
		}
	}
}

func TestExtractsPythonHeredocAsLanguageFrame(t *testing.T) {
	directSource := "python3 - <<'PY'\nimport socket\nfor host in [\"github.com\"]:\n    print(host)\nPY\n"
	direct, directErr := formatShellDisplay(directSource, 46)
	if directErr != nil || len(direct.Frames) != 2 {
		t.Fatalf("direct Python heredoc should project: err=%v frames=%#v", directErr, direct.Frames)
	}
	source := `ssh macbook-air 'python3 - <<'"'"'PY'"'"'
import socket
for host in ["github.com", "cowboy.example"]:
    print(socket.getaddrinfo(host, 443))
PY'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	var python *shellFrame
	for index := range display.Frames {
		if display.Frames[index].Language == "python" {
			python = &display.Frames[index]
			break
		}
	}
	if python == nil || python.Label != "Python" {
		t.Fatalf("expected a labeled Python frame: %#v", display.Frames)
	}
	if !strings.Contains(python.Text, "import socket") || strings.Contains(python.Text, "PY") {
		t.Fatalf("heredoc boundaries must stay in Bash: %q", python.Text)
	}
}

func TestExtractsInlineScriptLanguages(t *testing.T) {
	tests := []struct {
		source   string
		language string
	}{
		{`python3 -c 'import json; print(json.dumps({"ready": True}))'`, "python"},
		{`nix develop -c env PYTHONUTF8=1 python3.12 -c 'import json; print(json.dumps({"ready": True}))'`, "python"},
		{`deno eval 'const value = {ready: true}; console.log(JSON.stringify(value))'`, "typescript"},
		{`node --input-type=module -e 'const value = {ready: true}; console.log(JSON.stringify(value))'`, "javascript"},
		{`perl -e 'use JSON::PP; print encode_json({ready => JSON::PP::true});'`, "perl"},
		{`ruby -e 'require "json"; puts JSON.generate({ready: true})'`, "ruby"},
		{`php -r '$value = ["ready" => true]; echo json_encode($value);'`, "php"},
		{`lua -e 'local value = {ready = true}; for key, item in pairs(value) do print(key, item) end'`, "lua"},
		{`awk '{ totals[$1] += $2 } END { for (key in totals) print key, totals[key] }' data`, "awk"},
		{`sed -e 's/(alpha|beta)/replacement/g; /failed/{p;q;}' file`, "sed"},
		{`rg -e 'ToolDetails(Browser|Sheet)|format(Shell|Embedded)Frame' web/src`, "regex"},
	}
	for _, test := range tests {
		display, err := formatShellDisplay(test.source, 46)
		if err != nil {
			t.Fatalf("%s: %v", test.language, err)
		}
		found := false
		for _, frame := range display.Frames {
			found = found || frame.Language == test.language
		}
		if !found {
			t.Fatalf("expected %s frame: %#v", test.language, display.Frames)
		}
	}
}

func TestKeepsTrivialEmbeddedProgramsInline(t *testing.T) {
	for _, source := range []string{`python3 -c 'print(1)'`, `rg 'todo' .`, `rg -o 'cowboy-v[0-9]+' sw.js`, `rg 'a(b|c)[0-9]+' .`, `sed 's/a/b/' file`, `awk '{print}' file`} {
		display, err := formatShellDisplay(source, 46)
		if err != nil {
			t.Fatal(err)
		}
		if len(display.Frames) != 1 {
			t.Fatalf("trivial embedded program should stay inline: %s: %#v", source, display.Frames)
		}
	}
}

func TestNestedComplexityDoesNotBubbleIntoTransparentWrappers(t *testing.T) {
	source := `ssh macbook-air 'bash -c "for i in {1..12}; do value=$(/Library/PrivilegedHelperTools/xyz.stormbird.device version --json); echo $i $value; if echo $value | grep -q df6f983; then exit 0; fi; sleep 15; done; exit 1"'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 {
		t.Fatalf("transparent bash wrapper should be promoted, not framed: %#v", display.Frames)
	}
	if display.Frames[1].Launcher != "ssh macbook-air" || !strings.Contains(display.Frames[1].Text, "for i in") {
		t.Fatalf("complex direct descendant should remain under the remote launcher: %#v", display.Frames)
	}
	if display.Frames[1].Depth != 1 {
		t.Fatalf("promoted descendant depth=%d, want 1: %#v", display.Frames[1].Depth, display.Frames)
	}
}

func TestNestedLanguageFramesHaveExplicitLabels(t *testing.T) {
	display, err := formatShellDisplay(`printf x | jq '.items[] | select(.ready) | {id, ready}'; psql db -c 'select id, payload #>> '{}' from events where deleted_at is null order by id'`, 46)
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]string{"bash": "Bash", "jq": "jq", "sql": "SQL"}
	seen := map[string]bool{}
	for _, frame := range display.Frames {
		if label, ok := want[frame.Language]; ok {
			seen[frame.Language] = true
			if frame.Label == label {
				continue
			}
			t.Fatalf("language %s label=%q, want %q: %#v", frame.Language, frame.Label, label, display.Frames)
		}
	}
	for language := range want {
		if !seen[language] {
			t.Fatalf("missing labeled %s frame: %#v", language, display.Frames)
		}
	}
}

func TestExtractsQuotedSSHRemotePipeline(t *testing.T) {
	source := `ssh -o BatchMode=yes macbook-air 'scutil --nc status "SFM" | grep -i -E "ephemeral|expiry|tailscale|hostname|control_url" | sed -E '\''s/(auth_key|password|token|secret)[^,}]*/\1=REDACTED/Ig'\'' | head -n 80'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) < 2 || display.Frames[1].Launcher != "ssh -o BatchMode=yes macbook-air" {
		t.Fatalf("quoted SSH pipeline should become a remote frame: %#v", display.Frames)
	}
	if !strings.Contains(display.Frames[1].Text, "scutil") || !strings.Contains(display.Frames[1].Text, "sed -E") {
		t.Fatalf("remote pipeline content was lost: %#v", display.Frames)
	}
}

func TestKeepsComplexFishRemotePayloadAsLanguageFrame(t *testing.T) {
	source := `ssh macbook-air 'for i in (seq 1 20); set state (scutil --nc status "SFM" | head -n 1); test "$state" = Connected; and break; sleep 1; end'`
	display, err := formatShellDisplay(source, 46)
	if err != nil {
		t.Fatal(err)
	}
	if len(display.Frames) != 2 || display.Frames[1].Language != "fish" || display.Frames[1].Label != "Fish" {
		t.Fatalf("complex non-Bash remote source should use a labeled safe fallback: %#v", display.Frames)
	}
}
