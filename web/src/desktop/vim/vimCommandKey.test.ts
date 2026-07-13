import { vimCommandKey } from "./vimCommandKey";

function key(code: string, init: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { code, ...init } as KeyboardEvent;
}

Deno.test("physical v and V stay stable under an IME input source", () => {
  if (vimCommandKey(key("KeyV", { key: "Process" })) !== "v") {
    throw new Error("physical v was not preserved");
  }
  if (vimCommandKey(key("KeyV", { key: "Process", shiftKey: true })) !== "V") {
    throw new Error("physical V was not preserved");
  }
});

Deno.test("all standard Vim visual exits reach codemirror-vim", () => {
  const exits = [
    vimCommandKey(key("Escape")),
    vimCommandKey(key("BracketLeft", { ctrlKey: true })),
    vimCommandKey(key("KeyC", { ctrlKey: true })),
  ];
  const expected = ["<Esc>", "<C-[>", "<C-c>"];
  if (JSON.stringify(exits) !== JSON.stringify(expected)) {
    throw new Error(`visual exits differ: ${JSON.stringify(exits)}`);
  }
});

Deno.test("navigation and control commands use Vim tokens", () => {
  const commands = [
    vimCommandKey(key("ArrowLeft")),
    vimCommandKey(key("Home")),
    vimCommandKey(key("PageDown")),
    vimCommandKey(key("Backspace")),
    vimCommandKey(key("Delete")),
    vimCommandKey(key("Enter")),
    vimCommandKey(key("Space")),
    vimCommandKey(key("Space", { shiftKey: true })),
    vimCommandKey(key("Backspace", { shiftKey: true })),
    vimCommandKey(key("Tab")),
    vimCommandKey(key("KeyD", { ctrlKey: true })),
  ];
  const expected = [
    "<Left>",
    "<Home>",
    "<PageDown>",
    "<BS>",
    "<Del>",
    "<CR>",
    "<Space>",
    "<S-Space>",
    "<S-BS>",
    "<C-i>",
    "<C-d>",
  ];
  if (JSON.stringify(commands) !== JSON.stringify(expected)) {
    throw new Error(`Vim token mapping differs: ${JSON.stringify(commands)}`);
  }
});

Deno.test("desktop and browser shortcuts are not captured", () => {
  if (vimCommandKey(key("KeyV", { metaKey: true })) !== null) {
    throw new Error("Command-V must remain native paste");
  }
  if (vimCommandKey(key("KeyV", { altKey: true })) !== null) {
    throw new Error("Alt-modified keys must remain native");
  }
});
