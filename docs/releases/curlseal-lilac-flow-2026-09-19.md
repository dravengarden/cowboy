# Curlseal / Lilac Flow branding release

The approved Curlseal silhouette is now the official Cowboy mark. The default is
**26 · Lilac Flow**, with one shared `#D8C0FF` → `#9776DB` gradient across the
crown and spiral, on `#211B34` for installation icons. Transparent website,
README and browser marks use the same contours and pigments.

Settings exposes all 50 approved v3 styles in Flat and Flow groups of 25.
Preview, apply, persisted selection and restore-default are supported. Existing
legacy icon selections remain valid. System appearance and reading-font defaults
are retained; error, warning, success and info colors are unchanged. Website
headings use a darker lilac range on light surfaces for legibility.

## Published and active

- Brand commit: `c39a0c0b`; responsive heading refinement and release revision:
  `ed870bd3342312060f307ae8f48873159067d309`, pushed to remote `main`.
- Immutable Web release:
  `/nix/store/49ws0s01kh84il32xqqihr4dvghscax0-cowboy-web-release`.
- Hawk Web transaction: `1789807084870255567-ed870bd33423`, outcome `succeeded`,
  phase `committed`, published `true`, maintenance `false`.
- `/run/cowboy-web` points at that release. `/healthz` returned `ok`; `/version`
  and the SPA ETag both returned `78e5e670596be11d547e59c646957c6c`.
  The SPA uses `Cache-Control: no-store`; the served worker is `cowboy-v1736`.
- Controller and user-owned Machine remain active. No Controller or Machine
  activation was performed.
- [Website deployment](https://github.com/dravengarden/cowboy/actions/runs/35432518953)
  completed successfully for the release revision. The public homepage advertises
  Lilac Flow and 50 styles. Its served stylesheet and favicon match local bytes.

Verified SHA-256:

| Resource | SHA-256 |
| --- | --- |
| Default 512px App icon | `f01f3c5191c1965b1ecd6bcc0bcee9cdeb25438143e02af8b1b0a7899ebbfadf` |
| Website tab SVG | `c4b5f5866cc877c68a6cdafe4115e08216e3faff32f13dbf67882c3159d9f28d` |
| Website CSS | `c761423c060193d018c51bed5ee34a13ba590e5ccacebbb4a61835c6ab09c2da` |

## Validation

- Pinned Web typecheck and lint passed (three existing telemetry spread warnings).
- 18 focused icon/theme/resource/default tests passed, including all 50 styles in
  both modes, semantic colors, native asset inventory and ICO frame sizes.
- `just native-shell-check` passed, including its 29 Deno tests, Python checks,
  keyboard geometry and source/dependency validation.
- `just site-check` passed, including 12 tests and production website generation.
- Real Chromium website checks at 390px and 1280px in light and dark: no horizontal
  overflow, duplicate SVG IDs or broken images; manual mode and Chinese switching
  passed. Mobile headings and transparent header framing were visually reviewed.
- A temporary browser fixture using the actual AppIconSettings component verified
  25 options per group, apply, persistence after reload, restore-default,
  favicon/manifest/Apple-touch links and mobile width. The fixture is not shipped.
- The clean committed Nix Web release build passed.

## Native delivery boundary

Primary and all 50 alternate iOS source assets, bridge default, Android and desktop
assets are updated. This release does **not** include a newly built native binary:
SSH to the configured Mac build host and overlay endpoint failed. Native build,
signing and distribution remain pending a reachable Mac. Existing installed iOS
shells cannot gain new alternate icons from a Web deployment.

Installed iOS PWAs may retain their original Home Screen artwork; the Settings
installation guidance describes the platform-specific update/re-add flow. Reload
Cowboy after the update prompt to load the new Web bundle.
