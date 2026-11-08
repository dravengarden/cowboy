# machine-browser Plugin

A signed Chromium runtime that Cowboy core runs on a Linux Machine so the
owner can sign in to a Provider account from the Machine's own network
identity, operated from the phone or desktop. One instance per account, no
state shared between accounts, nothing resident when idle.

The Plugin ships bytes and a closed contract (`contract.json`): the
artifact-relative launcher, browser, display server and helpers, the managed
browser policy file and the directory the packaged browser build reads it
from, and the host requirements core checks before advertising the
capability. Admission, namespaces, the display stream, snapshots and the
viewer are core (`docs/machine-browser.md`). Without this Plugin installed, a
Machine has no browser capability.

- `runtime/` builds the artifact: the complete dependency closure of the
  browser, `Xtigervnc`, the window manager, the helpers, XKB data, fonts and
  glibc from the pinned Ubuntu release, with every symlink replaced by a copy
  and every ELF object audited against the tree alone. Inside an instance the
  artifact's `usr/` and `opt/` are the whole userland.
- `launcher/` is the static supervisor binary (`cowboy-browser-launcher`):
  namespaces, mounts, sealing, the three processes, the control protocol and
  snapshot copies. Its crate version is the Plugin version.
- `policy.json` is the managed browser policy bound into every instance.

`just plugin-check` validates the contract against the registry and the
launcher crate version; the runtime conformance runs the exact signed archive
through the real Machine installer on a pristine Ubuntu 26.04 guest.
