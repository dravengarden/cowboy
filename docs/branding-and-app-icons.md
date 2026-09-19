# Cowboy brand and app icons

The official default is **26 · Lilac Flow** (`curlseal-026`) from the approved
Curlseal v3 color study. The hat crown and spiral brim share one horizontal
lilac gradient, `#D8C0FF` → `#9776DB`, on an opaque `#211B34` installation tile.
The common silhouette is traced from the approved Curlseal shape; every export
uses the same paths and global gradient coordinates. No per-part gradient reset,
lighting, drop shadow or baked-in rounded corners is added.

## Appearance

**Settings → Appearance → Icon & theme → Choose** offers all **50** colorways:
**Flat 01–25** use one solid pigment across both pieces; **Flow 26–50** use one
continuous gradient. Numbers separated by 25 are paired color families.
The preview is separate from **Use this style**; **Restore default** selects 26.
System / Light / Dark and the reading-font preference are preserved. New installs
retain System and Source Serif 4 defaults. Existing deliberate icon choices remain
valid; the saved `default` sentinel follows the new brand.

Only primary and secondary accents vary with an icon choice. The theme engine
adjusts UI text/button colors for contrast on each mode. Backgrounds, neutral
states and the error, warning, success and info palettes are unchanged. Artwork
keeps the approved colors; UI contrast adjustment never recolors the logo.

## Surfaces and installation

- Website header/footer and README use transparent vector marks, with compact
  proportional framing and the original gradient. Light and dark surfaces share
  the same brand pigments. Website accents, focus states and prominent actions
  follow the lilac family; semantic/provider identity colors stay independent.
- Browser tabs use a separate optically centered transparent vector frame,
  with a fine contrast edge for pale/dark marks on browser chrome. ICO fallback
  contains actual 16/32/48px frames. App choices also update their tab favicon.
- PWA tiles use versioned `/app-icons/v10/` assets, padded maskable exports and
  per-style manifests/install pages. Manifest identity and scope remain `/`.
  iPhone/iPad Safari Home Screen icons are installation snapshots: open the
  selected installation page and Share → Add to Home Screen. Existing icons
  are not promised to change automatically. Chromium may offer an app identity
  update; other browsers may require adding the app again.
- Native iPhone/iPad icon switching requires a binary containing these assets.
  The trusted-origin bridge reports bundled inventory and commits only after
  UIKit reports success. An older binary can apply the theme separately and
  download the artwork. The primary icon and all 50 alternatives are exported
  in source; Web deployment alone does not update an installed native binary.
- Legacy `palette-*` and `original-*` assets and native alternate identifiers
  remain intact for existing saved choices, but are not listed in the new picker.
  The archived Neon alternate retains its light/dark appearance assets. Lilac Flow
  retains its selected opaque background in both modes.

## Reproducible assets

`assets/brand/cowboy-curlseal-contours.json` records the traced paths and source
hash. `assets/brand/cowboy-curlseal-palettes.json` owns all 50 approved color
recipes. `tools/build-curlseal-icons.py` exports the catalog, picker groups,
SVG/PNG tiles, manifests, installation pages, tab SVG/ICO, website/README marks,
and Apple/Android/desktop native artwork. Production does not read exploratory
session directories or call image generation. `tools/build-brand-icons.py`
delegates to this exporter while Curlseal is the active brand.

Run from the repository root in the pinned shell, with ImageMagick on PATH:

```sh
nix develop -c python3 tools/build-curlseal-icons.py
```

The PNGs are rasterized from vectors, rather than enlarged small thumbnails.
Platform launchers apply their own corner masks. Main native artwork is 1024px;
the tiny browser mark has independent padding, without distorting the silhouette.
Tests cover all 100 style/mode combinations for readable accents and unchanged
semantic colors, installation identity, legacy choices, catalog assets and tab
frames. Native source checks do not claim an Apple build or device acceptance.
