import { SvgIcon, type SvgIconProps } from "@mui/material";
import { useId } from "react";
import {
  providerPresentationEntry,
  useProviderCatalog,
} from "./providerCatalog";

/** Render only the asset supplied by the installed Provider contract. */
export function ProviderIcon({
  provider,
  providerVersion,
  providerDigest,
  ...props
}: SvgIconProps & {
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
}): React.JSX.Element | null {
  const { catalog } = useProviderCatalog();
  const entries = catalog?.providers ?? [];
  const presentationEntry = providerPresentationEntry(
    entries,
    provider,
    providerVersion,
    providerDigest,
  );
  const manifest = presentationEntry?.manifest;
  const asset = manifest?.ui.assets.find((candidate) =>
    candidate.id === manifest.display.icon_asset
  );
  const gradientId = `provider-icon-gradient-${useId().replaceAll(":", "")}`;
  if (!asset || !manifest) return null;
  if (asset.content.kind === "vector_path") {
    const gradient = asset.content.gradient;
    return (
      <SvgIcon
        viewBox={asset.content.view_box}
        role="img"
        aria-label={asset.accessible_label}
        {...props}
      >
        {gradient
          ? (
            <defs>
              <linearGradient
                id={gradientId}
                x1={`${gradient.x1_percent}%`}
                y1={`${gradient.y1_percent}%`}
                x2={`${gradient.x2_percent}%`}
                y2={`${gradient.y2_percent}%`}
              >
                {gradient.stops.map((stop) => (
                  <stop
                    key={`${stop.offset_percent}:${stop.color}`}
                    offset={`${stop.offset_percent}%`}
                    stopColor={stop.color}
                  />
                ))}
              </linearGradient>
            </defs>
          )
          : null}
        <path
          d={asset.content.path}
          fill={gradient
            ? `url(#${gradientId})`
            : asset.content.fill ?? "currentColor"}
        />
      </SvgIcon>
    );
  }
  return (
    <SvgIcon
      viewBox="0 0 24 24"
      role="img"
      aria-label={asset.accessible_label}
      {...props}
    >
      <image
        href={`data:${asset.media_type};base64,${asset.content.base64}`}
        width="24"
        height="24"
      />
    </SvgIcon>
  );
}
