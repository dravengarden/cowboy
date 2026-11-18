{ pkgs, rustToolchain }:
# `cowboy-account-sandbox`: the root daemon that runs each Antigravity
# account's processes as its own static system identity in confined
# transient systemd units (docs/antigravity-real-accounts-design.md). It is
# built on a development host and installed on OVH by the host configuration,
# which builds nothing, so it is a fully static musl executable built with the
# exact pinned Rust toolchain from this crate's own Cargo lock, the same way
# as the Machine browser launcher.
let
  staticToolchain = rustToolchain.override {
    targets = [ "x86_64-unknown-linux-musl" ];
  };
  platform = pkgs.pkgsCross.musl64.makeRustPlatform {
    cargo = staticToolchain;
    rustc = staticToolchain;
  };
in
platform.buildRustPackage {
  pname = "cowboy-account-sandbox";
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  src = pkgs.lib.fileset.toSource {
    root = ./.;
    fileset = pkgs.lib.fileset.unions [
      ./Cargo.toml
      ./Cargo.lock
      ./src
      ./tests
    ];
  };
  cargoDeps = (platform.importCargoLock.override {
    # Change only the download endpoint, not Cargo's registry definitions or
    # any checksum.
    fetchurl = args: pkgs.fetchurl (args // {
      url = pkgs.lib.replaceStrings
        [ "https://crates.io/api/v1/crates/" ]
        [ "https://static.crates.io/crates/" ] args.url;
    });
  }) {
    lockFile = ./Cargo.lock;
  };
  # The downloaded compiler runs on GNU/Linux. Its build scripts and proc
  # macros must use the build-host linker, not the musl target cc.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "-C target-feature=+crt-static -C force-frame-pointers=yes";
  postFixup = ''
    if ${pkgs.binutils}/bin/readelf -l "$out/bin/cowboy-account-sandbox" | ${pkgs.gnugrep}/bin/grep -q INTERP; then
      echo 'account sandbox unexpectedly requires a dynamic loader' >&2
      exit 1
    fi
    if ${pkgs.binutils}/bin/readelf -d "$out/bin/cowboy-account-sandbox" | ${pkgs.gnugrep}/bin/grep -q NEEDED; then
      echo 'account sandbox unexpectedly requires shared libraries' >&2
      exit 1
    fi
  '';
  meta = {
    description = "Static root daemon confining each Antigravity account in its own system identity";
    license = pkgs.lib.licenses.mit;
    mainProgram = "cowboy-account-sandbox";
  };
}
