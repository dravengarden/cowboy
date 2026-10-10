// Loaded before every `bun test` file (see bunfig.toml). The Web tests run
// outside a browser, and Bun does not ship `FileReader`, which the attachment
// pipeline uses for the browser's built-in base64 and text reads. Provide the
// two read modes those tests reach; anything else stays absent on purpose so a
// test that needs a real DOM fails instead of passing against a fake one.

if (typeof globalThis.FileReader === "undefined") {
  class TestFileReader {
    result: string | null = null;
    error: Error | null = null;
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;

    readAsDataURL(blob: Blob): void {
      this.#read(async () => {
        const bytes = new Uint8Array(await blob.arrayBuffer());
        const type = blob.type || "application/octet-stream";
        return `data:${type};base64,${Buffer.from(bytes).toString("base64")}`;
      });
    }

    readAsText(blob: Blob): void {
      this.#read(() => blob.text());
    }

    #read(load: () => Promise<string>): void {
      load().then(
        (result) => {
          this.result = result;
          this.onload?.();
        },
        (error: unknown) => {
          this.error = error instanceof Error ? error : new Error(String(error));
          this.onerror?.();
        },
      );
    }
  }
  Object.defineProperty(globalThis, "FileReader", {
    configurable: true,
    writable: true,
    value: TestFileReader,
  });
}
