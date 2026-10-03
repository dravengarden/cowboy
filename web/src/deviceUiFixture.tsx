/** Disposable iOS editor acceptance surface. No account, server or agent. */
import { createRoot } from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { SurfaceProvider } from "./surface/SurfaceProfile";
import { useThemeMode } from "./theme";
import { useKeyboardInset } from "./keyboardInset";

export async function run(): Promise<void> {
  const fixture = { submissions: [] as unknown[], imageFetches: 0, errors: [] as string[] };
  Object.assign(globalThis, { deviceUiFixture: fixture,
    CowboyDeviceProof: { proof: async () => "fixture", resetChallenge() {}, install() {} },
  });
  const descriptor = { schema: "dravengarden.cowboy.product-sync-dataset/v1", dataset_id: `dataset-${"a".repeat(64)}`,
    user_id: "fixture-user", database_version: 2, outbox_contract: "atomic-delta-v1" };
  const session = { id: "fixture-session", provider: "codex", machine_id: "fixture-machine", cwd: "/fixture",
    title: "Device image verification", status: "running", origin: "web", created_at_ms: 1, updated_at_ms: 1 };
  const image = { type: "image", url: "/api/artifacts/fixture.svg", mimeType: "image/svg+xml" };
  let drafts = [{ id: "fixture-draft", text: "Saved image caption", cmid: "fixture-draft-cmid", content: [image, { type: "text", text: "Saved image caption" }] }];
  let queue: typeof drafts = [];
  let version = 1;
  let sequence = 0;
  const originalFetch = globalThis.fetch.bind(globalThis);
  globalThis.fetch = async (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input), location.href);
    if (url.pathname === "/api/artifacts/fixture.svg") {
      fixture.imageFetches++;
      return new Response('<svg xmlns="http://www.w3.org/2000/svg" width="480" height="240"><rect width="480" height="240" fill="#baa1ed"/><text x="40" y="125" font-size="30">Protected fixture image</text></svg>', { headers: { "Content-Type": "image/svg+xml" } });
    }
    if (url.pathname === "/api/sync/dataset") return Response.json(descriptor);
    if (url.pathname.endsWith("/bootstrap")) return Response.json({ messages: [
      { type: "snapshot", session_id: session.id, events: [], reached_start: true },
      { type: "sync_patch", state: `queue:${session.id}`, version, value: { queue, drafts }, confirmed: [] },
    ] });
    if (url.pathname.startsWith("/api/")) return Response.json({ files: [], items: [], methods: [] });
    return originalFetch(input, init);
  };
  class Socket {
    static OPEN = 1;
    static CONNECTING = 0;
    static CLOSED = 3;
    readyState = 0;
    protocol = "cowboy-sync-v1";
    onopen?: (event: Event) => void;
    onclose?: (event: CloseEvent) => void;
    onmessage?: (event: MessageEvent) => void;
    constructor() {
      setTimeout(() => {
        this.readyState = 1;
        this.onopen?.(new Event("open"));
        this.emit({ type: "sessions", sessions: [session] });
        this.emit({ type: "bootstrap_complete" });
      }, 20);
    }
    emit(data: unknown) { this.onmessage?.(new MessageEvent("message", { data: JSON.stringify(data) })); }
    close() { this.readyState = 3; this.onclose?.(new CloseEvent("close")); }
    send(raw: string) {
      const command = JSON.parse(raw);
      if (command.type === "connection_probe") this.emit(command);
      if (command.type === "submit") {
        fixture.submissions.push(command);
        this.emit({ type: "event", envelope: { session_id: session.id, seq: ++sequence, cmid: command.cmid,
          kind: "update", update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: command.text } } } });
        this.emit({ type: "event", envelope: { session_id: session.id, seq: ++sequence, kind: "turn_end", stop_reason: "EndTurn" } });
      }
      if (command.type === "remove_draft") drafts = drafts.filter((draft) => draft.id !== command.id);
      if (command.type === "clear_drafts") drafts = [];
      if (command.type === "clear_queue") queue = [];
      if (command.type === "edit_draft") drafts = drafts.map((draft) => draft.id === command.id ? { ...draft, text: command.text, content: command.content } : draft);
      if (command.type.includes("draft") || command.type.includes("queue") || command.type === "submit") {
        this.emit({ type: "sync_patch", state: `queue:${session.id}`, version: ++version, value: { queue, drafts }, confirmed: command.cmid ? [command.cmid] : [] });
      }
    }
  }
  Object.assign(globalThis, { WebSocket: Socket });
  window.addEventListener("error", (event) => fixture.errors.push(event.message));
  const [{ MobileComposer }, { ResourceLightbox }, { bindProductSyncPrincipal }, store] = await Promise.all([
    import("./mobile/MobileComposer"), import("./ResourceLightbox"), import("./productSyncIdentity"), import("./store"),
  ]);
  bindProductSyncPrincipal("fixture-user");
  function App() {
    const { theme } = useThemeMode();
    useKeyboardInset();
    return <ThemeProvider theme={theme}><CssBaseline /><SurfaceProvider>
      <p style={{ margin: 20 }}>Isolated device image / editor fixture</p>
      <MobileComposer sessionId={session.id} status="running" />
      <ResourceLightbox />
    </SurfaceProvider></ThemeProvider>;
  }
  createRoot(document.getElementById("root")!).render(<App />);
  store.openSession(session.id);
}
