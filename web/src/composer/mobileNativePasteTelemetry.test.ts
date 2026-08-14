import { assertEquals } from "jsr:@std/assert";
import { mobileNativePasteInventory } from "./mobileNativePasteTelemetry";

const editorSource = await Deno.readTextFile(
  new URL("../ComposerEditor.tsx", import.meta.url),
);
const textareaSource = await Deno.readTextFile(
  new URL("../ComposerTextarea.tsx", import.meta.url),
);
const telemetrySource = await Deno.readTextFile(
  new URL("./mobileNativePasteTelemetry.ts", import.meta.url),
);

Deno.test("native paste inventory counts files and item-only entries", () => {
  const file = { name: "shot.png" } as File;
  const items = [
    { kind: "string" },
    { kind: "file", getAsFile: () => file },
  ] as unknown as DataTransferItemList;
  assertEquals(
    mobileNativePasteInventory({
      files: [file] as unknown as FileList,
      items,
    }),
    { listedFiles: 1, itemCount: 2, itemFiles: 1, fileCount: 1 },
  );
  assertEquals(
    mobileNativePasteInventory(null),
    { listedFiles: 0, itemCount: 0, itemFiles: 0, fileCount: 0 },
  );
});

Deno.test("native paste telemetry is content-free and wired into both editors", () => {
  assertEquals(telemetrySource.includes("mobile_native_paste_event"), true);
  assertEquals(telemetrySource.includes("listed_files:"), true);
  assertEquals(telemetrySource.includes("file.name"), false);
  assertEquals(editorSource.includes("reportMobileNativePasteEvent"), true);
  assertEquals(textareaSource.includes("reportMobileNativePasteEvent"), true);
  assertEquals(editorSource.includes('surface: "cm6"'), true);
  assertEquals(textareaSource.includes('surface: "textarea"'), true);
});
