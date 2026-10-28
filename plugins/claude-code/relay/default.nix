{ pkgs, rustToolchain }:
# The Claude Plugin's native MCP relay. Plugin runtime bytes cannot rely on
# the building host's Nix loader: build a static musl executable, as the Zed
# Plugin's portable adapter does, from this crate's own Cargo lock.
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
  pname = "cowboy-mcp-relay";
  version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  src = pkgs.lib.fileset.toSource {
    root = ./.;
    fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ];
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
  # The downloaded compiler runs on GNU/Linux. Its build scripts must use the
  # build-host linker, not the musl target cc.
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "-C target-feature=+crt-static";
  postFixup = ''
    if ${pkgs.binutils}/bin/readelf -l "$out/bin/cowboy-mcp-relay" | ${pkgs.gnugrep}/bin/grep -q INTERP; then
      echo 'portable relay unexpectedly requires a dynamic loader' >&2
      exit 1
    fi
    if ${pkgs.binutils}/bin/readelf -d "$out/bin/cowboy-mcp-relay" | ${pkgs.gnugrep}/bin/grep -q NEEDED; then
      echo 'portable relay unexpectedly requires shared libraries' >&2
      exit 1
    fi
  '';
  meta.mainProgram = "cowboy-mcp-relay";
}
