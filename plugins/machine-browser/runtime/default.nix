{ pkgs, rustToolchain }:
# The Machine browser Plugin's launcher (`cowboy-browser-launcher`): the
# static supervisor shipped inside the runtime artifact at
# `usr/libexec/cowboy-browser-launcher`. Plugin runtime bytes cannot rely on
# the building host's Nix loader, and this binary runs before the artifact's
# own glibc is mounted, so it is a fully static musl executable built with the
# exact pinned Rust toolchain from this crate's own Cargo lock, as the Zed and
# Claude Plugins build their portable binaries.
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
  pname = "cowboy-browser-launcher";
  version = (builtins.fromTOML (builtins.readFile ../launcher/Cargo.toml)).package.version;
  src = pkgs.lib.fileset.toSource {
    root = ../launcher;
    fileset = pkgs.lib.fileset.unions [
      ../launcher/Cargo.toml
      ../launcher/Cargo.lock
      ../launcher/src
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
    lockFile = ../launcher/Cargo.lock;
  };
  # The downloaded compiler runs on GNU/Linux. Its build scripts and proc
  # macros must use the build-host linker, not the musl target cc.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "-C target-feature=+crt-static -C force-frame-pointers=yes";
  postFixup = ''
    if ${pkgs.binutils}/bin/readelf -l "$out/bin/cowboy-browser-launcher" | ${pkgs.gnugrep}/bin/grep -q INTERP; then
      echo 'portable launcher unexpectedly requires a dynamic loader' >&2
      exit 1
    fi
    if ${pkgs.binutils}/bin/readelf -d "$out/bin/cowboy-browser-launcher" | ${pkgs.gnugrep}/bin/grep -q NEEDED; then
      echo 'portable launcher unexpectedly requires shared libraries' >&2
      exit 1
    fi
  '';
  meta = {
    description = "Static supervisor of one Cowboy Machine browser instance";
    license = pkgs.lib.licenses.mit;
    mainProgram = "cowboy-browser-launcher";
  };
}
