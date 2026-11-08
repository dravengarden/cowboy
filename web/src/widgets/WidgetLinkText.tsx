import { Link } from "@mui/material";
import type { ReactNode } from "react";
import { openExternalUrl, safeExternalUrl } from "../openExternal";

/** An `https:` reference; anything else renders as plain text. */
export function WidgetLinkText({
  url,
  children,
}: {
  url: string;
  children: ReactNode;
}): React.JSX.Element {
  const target = url.startsWith("https:") ? safeExternalUrl(url) : null;
  if (!target) return <>{children}</>;
  return (
    <Link
      href={target}
      underline="hover"
      onClick={(event): void => {
        event.preventDefault();
        openExternalUrl(target);
      }}
    >
      {children}
    </Link>
  );
}
