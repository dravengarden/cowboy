/** Compile-only boundary tests, never imported by the product. */
import type { CapturedContent, ContentObservation } from "./content.ts";
import type { OwnedCodeBuffer } from "./owner.ts";
import type { CodeBufferCleanup } from "./cleanup.ts";

export function cleanupTypes(cleanup: CodeBufferCleanup) {
  // @ts-expect-error a display resource ID is not an original cleanup handle
  cleanup.inspect("resource-id");
  // @ts-expect-error arbitrary objects cannot acquire cleanup authority
  cleanup.continueCleanup({ ordinal: 1 });
  // @ts-expect-error Settings cannot open a resource or revive a previous page
  cleanup.open("old-resource");
}

export function contentTypes(owner: OwnedCodeBuffer, content: CapturedContent) {
  const hover: Promise<ContentObservation<"hover">> = owner.readContent(
    content,
    { kind: "hover", position: { row: 0, column: 0 } },
  );
  const symbols: Promise<ContentObservation<"symbols">> = owner.readContent(
    content,
    { kind: "symbols" },
  );
  const language: Promise<ContentObservation<"language">> = owner.readContent(
    content,
    { kind: "language" },
  );
  // @ts-expect-error content capture cannot be supplied as JSON or a disk ETag
  owner.readContent({ text: "unowned", sha256: "digest" }, { kind: "symbols" });
  // @ts-expect-error a hover must name a position in the captured text
  owner.readContent(content, { kind: "hover" });
  owner.readContent(content, {
    // @ts-expect-error no destination ownership or generic operation escape hatch
    kind: "definition",
    position: { row: 0, column: 0 },
  });
  // @ts-expect-error query result types cannot be exchanged
  const wrong: Promise<ContentObservation<"symbols">> = hover;
  return { hover, symbols, language, wrong };
}
