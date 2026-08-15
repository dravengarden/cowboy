import { SvgIcon, type SvgIconProps } from "@mui/material";
import {
  providerEntryForIdentity,
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
  const manifest = providerEntryForIdentity(
    entries,
    provider,
    providerVersion,
    providerDigest,
  )?.manifest;
  const asset = manifest?.ui.assets.find((candidate) =>
    candidate.id === manifest.display.icon_asset
  );
  if (!asset || !manifest) return null;
  if (asset.content.kind === "vector_path") {
    return (
      <SvgIcon
        viewBox={asset.content.view_box}
        role="img"
        aria-label={asset.accessible_label}
        {...props}
      >
        <path
          d={asset.content.path}
          fill={asset.content.fill ?? manifest.display.accent}
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
