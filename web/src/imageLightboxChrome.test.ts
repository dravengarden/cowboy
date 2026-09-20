import { assert, assertEquals } from "jsr:@std/assert";

const lightboxSource = await Deno.readTextFile(
  new URL("../../components/app-shell/image-lightbox.tsx", import.meta.url),
);
const gestureSource = await Deno.readTextFile(
  new URL(
    "../../components/app-shell/image-lightbox-gestures.ts",
    import.meta.url,
  ),
);
const detentSheetSource = await Deno.readTextFile(
  new URL("../../components/app-shell/detent-sheet.tsx", import.meta.url),
);

Deno.test("the baked pan layer scales the plate's padding with it", () => {
  // The plate's padding lives inside the element's border box, so a transform
  // scales it with the artwork. Baking the scale into width/height while the
  // padding stayed at its CSS size widened the content box, and the diagram
  // jumped outwards on the frame the bake landed — the twitch at the end of a
  // pinch. The bake scales it; unbaking and reset hand it back to CSS.
  const bake = gestureSource.slice(
    gestureSource.indexOf("const bakePanLayer"),
    gestureSource.indexOf("const schedulePanLayer"),
  );
  assert(bake.includes("img.style.padding = `${box.padding * tf.current.scale}px`"));
  const unbake = gestureSource.slice(
    gestureSource.indexOf("const unbakeScale"),
    gestureSource.indexOf("const bakePanLayer"),
  );
  assert(unbake.includes('img.style.padding = ""'));
  const reset = gestureSource.slice(
    gestureSource.indexOf("const reset = useCallback"),
    gestureSource.indexOf("// Zoom by `factor`"),
  );
  assert(reset.includes('img.style.padding = ""'));
  // A baked layer already carries `padding × bakedScale`, so the measurement
  // divides it back out instead of compounding on the next zoom step.
  assert(gestureSource.includes("globalThis.getComputedStyle(img).paddingTop"));
  assert(gestureSource.includes(") / bakedScale.current"));
});

Deno.test("swapping the layer's layout commits before an animated transform", () => {
  // A transition starts from the previous style recalculation's value. Baking
  // (or unbaking) the layout and starting an animated transform in the same
  // task made the browser interpolate the OLD transform against the NEW layout:
  // at 3x the figure flashed to 9x and eased back over the settle — the twitch
  // users see when a pinch ends. Both size swaps repaint neutrally and commit
  // that paint, so the transition only has the translation left to run.
  const bake = gestureSource.slice(
    gestureSource.indexOf("const bakePanLayer"),
    gestureSource.indexOf("const schedulePanLayer"),
  );
  assert(bake.indexOf("paintTransform(false, true)") < bake.indexOf("commitPaint()"));
  const unbake = gestureSource.slice(
    gestureSource.indexOf("const unbakeScale"),
    gestureSource.indexOf("const bakePanLayer"),
  );
  assert(unbake.indexOf("bakedScale.current = 1") < unbake.indexOf("paintTransform()"));
  assert(unbake.indexOf("paintTransform()") < unbake.indexOf("commitPaint()"));
  // The commit is a forced read, so it must stay on gesture boundaries.
  assertEquals(
    gestureSource.split("commitPaint()").length - 1,
    2,
    "commitPaint is called by the two size swaps and nowhere else",
  );
  // An animated return to fit hands the baked layer back first, or it starts
  // from a collapsed figure.
  const reset = gestureSource.slice(
    gestureSource.indexOf("const reset = useCallback"),
    gestureSource.indexOf("// Zoom by `factor`"),
  );
  assert(reset.indexOf("if (animate) {") < reset.indexOf("unbakeScale();"));
  assert(reset.indexOf("unbakeScale();") < reset.indexOf('img.style.width = ""'));
});

Deno.test("a fullscreen preview takes the standalone status bar with it", () => {
  // An iOS standalone PWA paints its status bar from <meta name="theme-color">
  // and that strip sits above the web view, so the backdrop cannot cover it. A
  // light app otherwise keeps a bright band over the near-black preview.
  assert(detentSheetSource.includes("export function setStatusBarColor"));
  assert(lightboxSource.includes('import { setStatusBarColor } from "./detent-sheet.tsx"'));
  assert(lightboxSource.includes('const BACKDROP_COLOR = "#0b0b0e"'));
  assert(lightboxSource.includes("backgroundColor: BACKDROP_COLOR"));
  const effect = lightboxSource.slice(
    lightboxSource.indexOf("const previous = globalThis.document?.head"),
    lightboxSource.indexOf("// Key shortcuts while open"),
  );
  assert(effect.includes("setStatusBarColor(BACKDROP_COLOR)"));
  // Restore on close, and never write a colour we did not read.
  assert(effect.indexOf("return () => {") < effect.indexOf("setStatusBarColor(previous)"));
  assertEquals(effect.includes("document.body.style"), false);
});
