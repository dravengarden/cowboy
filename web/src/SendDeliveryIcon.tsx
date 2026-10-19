import { SvgIcon, type SvgIconProps } from "@mui/material";

/** Material Send's filled silhouette, with softened wing tips and a clean fold. */
export function SendDeliveryIcon(props: SvgIconProps): React.JSX.Element {
  return (
    <SvgIcon {...props} viewBox="0 0 24 24">
      <path d="M3.15 3.62a.65.65 0 0 0-.9.61v5.4c0 .3.22.56.52.61L16.5 12 2.77 13.76c-.3.04-.52.3-.52.61v5.4c0 .47.48.78.9.6l18.76-7.76a.66.66 0 0 0 0-1.22L3.15 3.62Z" />
    </SvgIcon>
  );
}
