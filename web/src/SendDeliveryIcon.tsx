import { SvgIcon, type SvgIconProps } from "@mui/material";

/** One optical footprint for Send and its delivery-menu variant. */
export function SendDeliveryIcon({
  menu = false,
  ...props
}: SvgIconProps & { readonly menu?: boolean }): React.JSX.Element {
  return (
    <SvgIcon {...props} viewBox="0 0 24 24">
      <path
        d="M3.5 9.75 20.5 3.5 14.25 20.5 10.5 13.25 3.5 9.75ZM10.5 13.25 20.5 3.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {menu && (
        <path
          data-delivery-chevron
          d="m17.5 17.5 2.25 2.25L22 17.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      )}
    </SvgIcon>
  );
}
