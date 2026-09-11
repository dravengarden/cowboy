# Cowboy brand and app icons

The default is colorway **54**: rose crown `#E8BDD0`, ice-blue brim `#BDD2ED`,
and charcoal background `#232831`. It uses the approved Boomerang West shape.
The website and app accents follow these colors; CSS must not recolor the logo
with hue rotation, a mask, or a rainbow overlay. The light theme uses deeper blue
and rose accents so text remains readable against pale surfaces.

## Choosing an icon

Open **Settings → Appearance → App icon → Choose**. The library contains 200
colorways: the original 50 (prefixed `O`) and subsequent colorways 1–150. Search
by number, name, or hex color, filter either hat color and background brightness,
preview, then choose **Use this icon**. The grid loads at most 24 thumbnails per
page. **Restore default** selects 54. A preview is not a committed selection.

| Surface | What changes | Completing the Home Screen / app icon change |
| --- | --- | --- |
| Browser tab | Favicon and the selected installation manifest | Selection is saved in this browser; other browsers/devices keep their own selection. |
| iPhone / iPad Safari web app | Favicon and installation artwork | Open the selected installation page in Safari, Share → Add to Home Screen. Existing Home Screen artwork is not promised to update in place. |
| Chromium installed web app | Versioned manifest icon URLs | Recent Chrome presents **Review app update** in its menu for identity changes. The user/browser controls acceptance; older browsers may need reinstallation. |
| Safari on macOS | Selected installation artwork | Add the selected page to the Dock again if the installed icon does not update. |
| Native iPhone / iPad | System alternate app icon | Requires a native build containing the requested icon. The UI commits only after UIKit reports that icon as current; rejection leaves the preference unchanged. |
| Older native apps / other native platforms | Preview and downloadable PNG | Automatic switching is unavailable unless the shell advertises it. Use system icon controls where available, or update the iOS app. |

No action deletes an installed app, its sessions, browser data, or login.
Installation pages carry their selection through `start_url`; all manifests keep
the same `id: "/"` and `scope: "/"`. A consumed installation handoff must not
overwrite a later choice on every launch. The page also has a static Apple touch
icon, so Safari need not rely on a React-time mutation to discover the artwork.
Blocked local storage degrades to the current window; it must never crash startup.

Platform references:

- [Apple: webpage-specific Home Screen icons](https://developer.apple.com/library/archive/documentation/AppleApplications/Reference/SafariWebContent/ConfiguringWebApplications/ConfiguringWebApplications.html).
- [WebKit: Apple touch icons take precedence over manifest icons](https://webkit.org/blog/13878/web-push-for-web-apps-on-ios-and-ipados/).
- [Chrome 144: user-reviewed icon updates and changed icon URLs](https://developer.chrome.com/blog/improvements-to-web-app-updates).
- [Apple: alternate app icons](https://developer.apple.com/documentation/xcode/configuring-your-app-to-use-alternate-app-icons).

## Asset ownership and export

`web/src/appIconCatalog.json` and the committed
`web/public/app-icons/v5/<id>/icon-512.png` files are the export inputs. The catalog
records the intended colors and the original generated file's SHA-256. The
source images have subtle shading; a listed hex color is the design target,
not a claim that every pixel has that exact RGB value. Production builds never
read old session directories or make image-generation calls.

Run from the repository root:

```sh
nix develop -c python3 tools/build-brand-icons.py
```

The exporter resizes approved artwork without redrawing its contours. It owns
web icons, padded maskable icons, Apple touch icons, legacy compatibility aliases,
static per-colorway installation pages/manifests, native icon sets, ICNS/ICO,
README artwork, and website favicons/brand artwork. Rounded masks are supplied
by the platform; the original artwork keeps its opaque background. Maskable
exports add safe space for circular/adaptive launcher masks.

To import another approved batch once, use `--import-batch palette=/path/to/batch`
with numbered JSON/PNG pairs (`--only-imported` skips re-exporting old variants).
Commit the resulting catalog, input PNGs, and exports together. The large
exploration files remain outside Git under `output/`.

## Native trust and release

`CowboyAppIconBridge.mm` is core product appearance, separate from the Plugin
capability ABI. Only the trusted HTTPS Cowboy main frame can query or change
icons. Requested IDs must match compiled `CFBundleAlternateIcons`; arbitrary
paths, URLs, and downloaded icon installation are not accepted. UIKit's
completion and actual `alternateIconName` are authoritative. Calls are serialized
and require the app to be active.

Xcode includes all alternate AppIcon assets. A new web deployment cannot add
icons to an already-installed binary; the web selector uses the installed
binary's available-icon inventory. Native distribution requires rebuilding and
publishing that binary separately from the web component. A successful Linux
asset check is not a claim of an Apple build or physical-device acceptance.

Web changes bump the service worker generation and use versioned icon URLs.
Website publication follows the GitHub Pages workflow. Native checks include
bridge coexistence and rejection of foreign-origin icon requests in an isolated
Simulator; physical-device appearance remains a separate acceptance check.

SideStore **0.1.29** is the first release containing all 200 colorways. The Apple
build verifies that both iPhone and iPad declare the expected 199 alternatives
plus the primary icon. Publishing makes the update available in SideStore;
installation and device acceptance remain separate steps.
