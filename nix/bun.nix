{ pkgs }:

# Keep Cowboy's JavaScript toolchain hermetic without importing another
# repository. nixpkgs trails the upstream release, so pin the official binary
# through nixpkgs' own packaging. Delete this override once nixpkgs supplies
# the exact supported Bun release.
pkgs.bun.overrideAttrs (_: rec {
  version = "1.4.3";
  src = pkgs.fetchurl {
    url = "https://github.com/oven-sh/bun/releases/download/bun-v${version}/bun-linux-x64-baseline.zip";
    hash = "sha256-H8LtrIQxApCeOhvh2NmALMYHHPB05n6IIx9P/g+LOXs=";
  };
})
