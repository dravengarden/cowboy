import { Box } from "@mui/material";

/**
 * Shared style for left-truncating long repository paths so the filename stays
 * visible. Always pair with `<ReviewPathText>` (`<bdi dir="ltr">`) so leading
 * neutral characters such as `.github/...` or `/...` stay on the left instead
 * of flipping to the end under `direction: "rtl"`.
 */
export const REVIEW_TRUNCATED_PATH_SX = {
  direction: "rtl",
  textAlign: "left",
} as const;

/** BiDi-isolated path text for use inside a `direction: "rtl"` container. */
export function ReviewPathText({ path }: { path: string }): React.JSX.Element {
  return <Box component="bdi" dir="ltr">{path}</Box>;
}
