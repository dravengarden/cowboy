import { assertEquals } from "jsr:@std/assert";
import {
  USER_BUBBLE_COLLAPSE_BUFFER_PX,
  USER_BUBBLE_COLLAPSE_PX,
  userBubbleShouldClamp,
} from "./userBubbleCollapse";

Deno.test("user bubbles stay unclamped until they clear the collapse buffer", () => {
  const justOverCap = USER_BUBBLE_COLLAPSE_PX + 40;
  assertEquals(
    userBubbleShouldClamp({
      measured: true,
      naturalHeight: justOverCap,
      expanded: false,
      containsImage: false,
    }),
    false,
  );
  assertEquals(
    userBubbleShouldClamp({
      measured: true,
      naturalHeight: USER_BUBBLE_COLLAPSE_PX + USER_BUBBLE_COLLAPSE_BUFFER_PX + 1,
      expanded: false,
      containsImage: false,
    }),
    true,
  );
});

Deno.test("unmeasured bubbles clamp so long content cannot flash open", () => {
  assertEquals(
    userBubbleShouldClamp({
      measured: false,
      naturalHeight: 0,
      expanded: false,
      containsImage: false,
    }),
    true,
  );
});

Deno.test("expanded and image bubbles never clamp", () => {
  assertEquals(
    userBubbleShouldClamp({
      measured: true,
      naturalHeight: 800,
      expanded: true,
      containsImage: false,
    }),
    false,
  );
  assertEquals(
    userBubbleShouldClamp({
      measured: false,
      naturalHeight: 800,
      expanded: false,
      containsImage: true,
    }),
    false,
  );
});
