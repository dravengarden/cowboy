import {
  BOOT_SNAPSHOT_CACHE,
  BOOT_SNAPSHOT_URL,
  BOOT_SNAPSHOT_VERSION,
  type BootSnapshot,
  captureBootSnapshot,
  clearBootSnapshot,
} from "./bootSnapshot";

/** Real DOM/Cache Storage: a stale screen must never impersonate a live draft. */
export async function runBootSnapshotBrowserConformance(): Promise<string[]> {
  const root = document.createElement("div");
  root.id = "root";
  document.body.append(root);
  const draftKey = "cowboy:composer-draft:fixture-session";
  const text = "此前已发送的文字，不应在启动时重现";
  const draft = JSON.stringify({ text, attachments: [] });
  localStorage.setItem(draftKey, draft);
  const cases = [
    ["native textarea", "textarea"],
    ["native input", "input"],
    ["contenteditable", "div[contenteditable]"],
    ["CodeMirror editor and attachment widgets", ".cm-editor"],
  ] as const;
  const results: string[] = [];
  try {
    for (const [name, selector] of cases) {
      root.innerHTML = "<p>Visible transcript stays</p><section></section>";
      const container = root.querySelector("section")!;
      container.innerHTML = selector === "textarea"
        ? "<textarea></textarea>"
        : selector === "input"
        ? "<input>"
        : selector === ".cm-editor"
        ? '<div class="cm-editor"><div class="cm-content" contenteditable="true"></div><img alt="old-attachment"></div>'
        : '<div contenteditable="true"></div>';
      const editor = container.firstElementChild as HTMLElement;
      editor.style.cssText = "box-sizing:border-box;width:280px;height:80px";
      if (
        editor instanceof HTMLTextAreaElement ||
        editor instanceof HTMLInputElement
      ) {
        editor.value = text;
      } else if (selector === ".cm-editor") {
        editor.querySelector(".cm-content")!.textContent = text;
      } else editor.textContent = text;
      const captured = await captureBootSnapshot({
        user: "fixture-user",
        sessionId: "fixture-session",
        themeMode: "system",
        busy: false,
      });
      if (!captured) throw new Error(`${name}: capture failed`);
      const cache = await caches.open(BOOT_SNAPSHOT_CACHE);
      const snapshot: BootSnapshot =
        await (await cache.match(BOOT_SNAPSHOT_URL))!.json();
      if (
        snapshot.v !== BOOT_SNAPSHOT_VERSION || snapshot.html.includes(text) ||
        snapshot.html.includes("old-attachment") ||
        !snapshot.html.includes("Visible transcript stays")
      ) {
        throw new Error(
          `${name}: snapshot kept editable content or lost transcript`,
        );
      }
      const saved = document.createElement("template");
      saved.innerHTML = snapshot.html;
      const placeholder = saved.content.querySelector("section")!
        .firstElementChild as HTMLElement;
      if (
        placeholder.tagName !== "DIV" || placeholder.style.width !== "280px" ||
        placeholder.style.height !== "80px" ||
        placeholder.childNodes.length !== 0
      ) {
        throw new Error(`${name}: editor geometry was not preserved`);
      }
      const liveText = editor instanceof HTMLTextAreaElement ||
          editor instanceof HTMLInputElement
        ? editor.value
        : editor.textContent;
      if (
        !liveText?.includes(text) || localStorage.getItem(draftKey) !== draft
      ) {
        throw new Error(`${name}: capture modified the live draft`);
      }
      results.push(name);
    }
    return results;
  } finally {
    root.remove();
    localStorage.removeItem(draftKey);
    clearBootSnapshot();
  }
}
