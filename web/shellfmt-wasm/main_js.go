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
	if len(args) != 2 || args[0].Type() != js.TypeString || args[1].Type() != js.TypeNumber {
		return mapResult(shellDisplay{}, fmt.Errorf("expected source and column width"))
	}
	display, err := formatShellDisplay(args[0].String(), args[1].Int())
	return mapResult(display, err)
}

func mapResult(display shellDisplay, err error) map[string]any {
	frames := make([]any, len(display.Frames))
	for index, frame := range display.Frames {
		frames[index] = map[string]any{"launcher": frame.Launcher, "text": frame.Text, "depth": frame.Depth, "marker": frame.Marker}
	}
	result := map[string]any{
		"text": display.Text, "flatText": display.FlatText, "context": display.Context,
		"frames": frames, "summary": display.Summary,
	}
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
