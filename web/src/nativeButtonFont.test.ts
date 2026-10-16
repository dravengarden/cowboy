import { test } from "bun:test";
import { assert } from "@std/assert";

const theme = await Bun.file(new URL("./theme.ts", import.meta.url)).text();

test("the native button font default never outranks component font sizes", () => {
  // Nested under ButtonBase's class it kept class specificity and beat every
  // Button, IconButton and sx font size, freezing them at 13.33px.
  assert(!theme.includes('"&:where(button)"'));
  const baseline = theme.slice(
    theme.indexOf("MuiCssBaseline"),
    theme.indexOf("MuiButtonBase"),
  );
  assert(
    baseline.includes('":where(button)": { fontSize: `${(40 / 3) / 16}rem` }'),
  );
});
