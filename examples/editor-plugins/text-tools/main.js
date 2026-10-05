// Text tools: a complete Cowboy editor plugin. It runs in a sandboxed Worker
// with no network or App access; everything goes through `ctx` and `editor`.

const pad = (n) => String(n).padStart(2, "0");

function timestamp(format, now) {
  const date = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${
    pad(now.getDate())
  }`;
  if (format === "date") return date;
  if (format === "iso") return now.toISOString();
  return `${date} ${pad(now.getHours())}:${pad(now.getMinutes())}`;
}

function words(text) {
  // CJK characters count individually; other scripts count by whitespace.
  const cjk = text.match(/[぀-ヿ㐀-鿿豈-﫿]/g)?.length ?? 0;
  const latin = text.replace(/[぀-ヿ㐀-鿿豈-﫿]/g, " ")
    .split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;
  return cjk + latin;
}

function selected(doc) {
  const from = Math.min(doc.selection.anchor, doc.selection.head);
  const to = Math.max(doc.selection.anchor, doc.selection.head);
  return doc.text.slice(from, to);
}

definePlugin({
  async onload(ctx) {
    const data = (await ctx.loadData()) ?? { sorts: 0 };

    ctx.addCommand({
      id: "sort-lines",
      title: "Sort selected lines",
      description: "Sort the selected lines using the plugin's sort settings",
      icon: "sort",
      toolbar: true,
      async run(editor) {
        const text = selected(editor.read());
        if (!text.includes("\n")) {
          ctx.notice("Select two or more lines to sort.");
          return;
        }
        const { order, caseSensitive } = ctx.settings;
        const key = (line) => (caseSensitive ? line : line.toLocaleLowerCase());
        const lines = text.split("\n").sort((a, b) =>
          key(a).localeCompare(key(b))
        );
        if (order === "desc") lines.reverse();
        if (await editor.replaceSelection(lines.join("\n"))) {
          data.sorts += 1;
          await ctx.saveData(data);
        } else {
          ctx.notice("The text changed; run Sort again.");
        }
      },
    });

    ctx.addCommand({
      id: "insert-timestamp",
      title: "Insert timestamp",
      icon: "clock",
      toolbar: true,
      async run(editor) {
        await editor.replaceSelection(
          timestamp(ctx.settings.timestamp, new Date()),
        );
      },
    });

    ctx.addCommand({
      id: "count-selection",
      title: "Count words in selection",
      icon: "calc",
      run(editor) {
        const doc = editor.read();
        const text = selected(doc) || doc.text;
        ctx.notice(`${words(text).toLocaleString()} words`);
      },
    });

    ctx.addPanel({
      id: "statistics",
      title: "Text statistics",
      render(doc) {
        const count = words(doc.text);
        const minutes = Math.max(
          1,
          Math.round(count / ctx.settings.wordsPerMinute),
        );
        return [
          { label: `${count.toLocaleString()} words` },
          { label: `${[...doc.text].length.toLocaleString()} characters` },
          { label: `About ${minutes} min to read` },
          {
            label: `Sorted ${data.sorts} times`,
            detail: "Saved with this plugin's data",
          },
        ];
      },
    });
  },
  onunload() {},
});
