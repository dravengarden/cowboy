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
	display, err := formatShellDisplay("nix develop -c bash -lc 'cargo fmt --check && cargo test'")
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
