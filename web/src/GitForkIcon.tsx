import { SvgIcon, type SvgIconProps } from "@mui/material";

/** Standard git-fork glyph (two branches diverging from a base node) at
 *  Material Outlined's 2px stroke weight so conversation-branch controls
 *  stay distinct from `AccountTreeOutlined` (background tasks / future
 *  orchestrator & swarm). */
export function GitForkIcon(props: SvgIconProps): React.JSX.Element {
  return (
    <SvgIcon
      {...props}
      data-icon="git-fork"
      viewBox="0 0 24 24"
      sx={[
        {
          fill: "none",
          stroke: "currentColor",
          strokeWidth: 2,
          strokeLinecap: "round",
          strokeLinejoin: "round",
        },
        ...(Array.isArray(props.sx) ? props.sx : [props.sx]),
      ]}
    >
      <circle cx="12" cy="18" r="2.75" />
      <circle cx="6" cy="6" r="2.75" />
      <circle cx="18" cy="6" r="2.75" />
      <path d="M18 8.75v1.25a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V8.75" />
      <path d="M12 12v3.25" />
    </SvgIcon>
  );
}
