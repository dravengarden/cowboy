package main

import (
	"strings"
	"testing"
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

func TestRejectsInvalidShell(t *testing.T) {
	if _, err := formatShellSource("echo '"); err == nil {
		t.Fatal("expected an incomplete quote to fail closed")
	}
}

func TestExtractsNixBashCommandPayload(t *testing.T) {
	display, err := formatShellDisplay("nix develop -c bash -lc 'cargo fmt --check && cargo test'", 80)
	if err != nil {
		t.Fatal(err)
	}
	if display.Context != "Nix develop → Bash -lc" {
		t.Fatalf("unexpected context %q", display.Context)
	}
	if strings.Contains(display.Text, "nix develop") || !strings.Contains(display.Text, "&&\n") {
		t.Fatalf("expected readable inner script, got %q", display.Text)
	}
}

func TestWrapsLongCallsAtShellWordBoundaries(t *testing.T) {
	display, err := formatShellDisplay("psql 'postgresql://cowboy?host=/run/postgresql-cowboy&port=5433&user=cowboy' -P pager=off -F $'\\t' -A -c 'select id, title from sessions order by updated_at desc'", 46)
	if err != nil {
		t.Fatal(err)
	}
	formatted := display.Text
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
