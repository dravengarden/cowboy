import { assertEquals } from "jsr:@std/assert";
import { reflowPerl } from "./perlFormatter.ts";
import { formatEmbeddedFrame } from "./shellFormatter.ts";

Deno.test("Perl substitutions become one statement per display line", () => {
  assertEquals(
    reflowPerl(String.raw`s#/assets/app-v2\.css#/assets/app-v3.css#g; s#/assets/app-v3\.js#/assets/app-v4.js#g`, 46),
    String.raw`s#/assets/app-v2\.css#/assets/app-v3.css#g;
s#/assets/app-v3\.js#/assets/app-v4.js#g`,
  );
});

Deno.test("Perl reflow preserves semicolons inside strings and quote-like operators", () => {
  assertEquals(
    reflowPerl(String.raw`print "alpha;beta"; s{old;value}{new;value}g; say q#done;still quoted#;`, 46),
    `print "alpha;beta";\ns{old;value}{new;value}g;\nsay q#done;still quoted#;`,
  );
});

Deno.test("embedded Perl frames use the display reflow", async () => {
  const frame = await formatEmbeddedFrame({
    launcher: "perl -e",
    language: "perl",
    label: "Perl",
    text: "use strict; print qq#ready;still quoted#; exit 0;",
  }, 46);
  assertEquals(frame.text, "use strict;\nprint qq#ready;still quoted#;\nexit 0;");
});
