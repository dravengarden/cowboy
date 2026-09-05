{ pkgs, rustToolchain, rustPlatform }:
# Private exact build recipes owned by the Zed Plugin. The root flake only
# exposes these outputs for both legacy bootstrap and Plugin publication.
let
  buildZedAdapter = platform: platform.buildRustPackage {
    pname = "cowboy-zed-adapter";
    version = (builtins.fromTOML
      (builtins.readFile ../adapter/Cargo.toml)).package.version;
    src = pkgs.lib.cleanSource ../adapter;
    cargoLock = {
      lockFile = ../adapter/Cargo.lock;
      outputHashes = {
        "proto-0.1.0" =
          "sha256-sAjiYGwmQB+Zzb/b7PGm4Nfv36Vb0myqKIBfpuHGTik=";
      };
    };
    nativeBuildInputs = [ pkgs.protobuf ];
    meta = {
      description = "GPL-isolated Zed protocol adapter for Cowboy Code";
      license = pkgs.lib.licenses.gpl3Plus;
      mainProgram = "cowboy-zed-adapter";
    };
  };

  cowboy-zed-adapter = buildZedAdapter rustPlatform;
  # Plugin runtime bytes cannot rely on the building host's Nix loader.
  # Keep the bootstrap output separate and build the distributable adapter
  # against musl with the same exact Rust toolchain and Cargo lock.
  zedStaticToolchain = rustToolchain.override {
    targets = [ "x86_64-unknown-linux-musl" ];
  };
  cowboy-zed-adapter-portable = (buildZedAdapter (pkgs.pkgsCross.musl64.makeRustPlatform {
    cargo = zedStaticToolchain;
    rustc = zedStaticToolchain;
  })).overrideAttrs (previous: {
    # The downloaded compiler runs on GNU/Linux. Its build scripts and
    # proc macros must use the build-host linker, not the musl target cc.
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc}/bin/cc";
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS = "-C target-feature=+crt-static -C force-frame-pointers=yes";
    postFixup = (previous.postFixup or "") + ''
      if ${pkgs.binutils}/bin/readelf -l "$out/bin/cowboy-zed-adapter" | ${pkgs.gnugrep}/bin/grep -q INTERP; then
        echo 'portable adapter unexpectedly requires a dynamic loader' >&2
        exit 1
      fi
      if ${pkgs.binutils}/bin/readelf -d "$out/bin/cowboy-zed-adapter" | ${pkgs.gnugrep}/bin/grep -q NEEDED; then
        echo 'portable adapter unexpectedly requires shared libraries' >&2
        exit 1
      fi
    '';
  });

  # Zed's official remote-development flow installs a release server on
  # the target host rather than compiling the editor workspace there.
  # Pin the exact preview release that corresponds to ZED_REVISION in the
  # adapter. This keeps Cowboy's instance reproducible and independent of
  # the user's ~/.zed_server lifecycle.
  cowboy-zed-server = pkgs.runCommand "cowboy-zed-server-1.13.0" {
    src = pkgs.fetchurl {
      url =
        "https://github.com/zed-industries/zed/releases/download/v1.13.0-pre/zed-remote-server-linux-x86_64.gz";
      hash = "sha256-+E10MkfNuSORMNvhyRm3Ij5UfM5mrWwKSVkj+FJGQ+Y=";
    };
    nativeBuildInputs = [ pkgs.gzip ];
    meta = {
      description = "Pinned isolated Zed remote server for Cowboy Code";
      license = pkgs.lib.licenses.gpl3Plus;
      mainProgram = "cowboy-zed-server";
    };
  } ''
      mkdir -p "$out/bin"
      gzip -dc "$src" > "$out/bin/cowboy-zed-server"
      chmod 0555 "$out/bin/cowboy-zed-server"
  '';
in {
  inherit cowboy-zed-adapter cowboy-zed-adapter-portable cowboy-zed-server;
}
