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
  - `lock.sh` resolves the closure in a throwaway rootless Docker
    `ubuntu:26.04` container and writes `packages.lock.json` (every .deb's
    URL, version, size and SHA-256) and the Chrome version into
    `contract.json`. Rerun it for a Chrome or Ubuntu update.
  - `build.ts` assembles the tree from the lock alone (no Docker), adds the
    launcher, `policy.json` and `nsswitch.conf`, audits, packs
    `dist/plugins/machine-browser/runtime-linux-x86_64.tar.gz` and writes the
    `runtime-artifacts.json` that `just plugin-bind-runtime` consumes:
    `bun plugins/machine-browser/runtime/build.ts <artifact-base-url>`.
  - The display server is the one non-Ubuntu binary: `display-server.ts`
    rebuilds `Xtigervnc` from Ubuntu's tigervnc source with
    `native-touch.patch` in the build root of Ubuntu's own build (pinned by
    `display-server.lock.json`, written by `display-server-lock.sh` from
    the Launchpad `.buildinfo` and `snapshot.ubuntu.com`) in a bubblewrap
    sandbox, and accepts only the recorded digest. `--record` records a
    reviewed output, `--verify-stock` proves the unpatched rebuild equals
    Ubuntu's binary. See "Native touch" in `docs/machine-browser.md`.
  - `default.nix` is the launcher's static musl build, exposed by the root
    flake as `cowboy-browser-launcher-portable`.
  - `conformance.sh` runs the built tree in a pristine `ubuntu:26.04`
    container as an unprivileged user (no `AppArmor`) and verifies the
    launcher contract from outside; `conformance_probe.py` is its Machine
    stand-in and `touch_probe.py` its native touch checks.
- `launcher/` is the static supervisor binary (`cowboy-browser-launcher`):
  namespaces, mounts, sealing, the three processes, the control protocol and
  snapshot copies. Its crate version is the Plugin version. The wire contract
  as implemented is `PROTOCOL.md`.
- `policy.json` is the managed browser policy bound into every instance.

`just plugin-check` validates the contract against the registry and the
launcher crate version; the runtime conformance runs the exact signed archive
through the real Machine installer on a pristine Ubuntu 26.04 guest.
