//go:build js && wasm

// Command shellfmt-wasm exposes mvdan/sh's Bash parser and printer to Cowboy's
// browser UI. It formats display copies only; Cowboy always executes and copies
// the original command bytes.
package main

import (
	"fmt"
	"syscall/js"
)

func formatShell(_ js.Value, args []js.Value) any {
	if len(args) != 1 || args[0].Type() != js.TypeString {
		return mapResult(shellDisplay{}, fmt.Errorf("expected one string argument"))
	}
	display, err := formatShellDisplay(args[0].String())
	return mapResult(display, err)
}

func mapResult(display shellDisplay, err error) map[string]any {
	result := map[string]any{"text": display.Text, "context": display.Context}
	if err != nil {
		result["ok"] = false
		result["error"] = err.Error()
		return result
	}
	result["ok"] = true
	result["error"] = ""
	return result
}

func main() {
	js.Global().Set("cowboyFormatShell", js.FuncOf(formatShell))
	select {}
}
