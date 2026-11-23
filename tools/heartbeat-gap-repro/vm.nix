# Disposable guest for the heartbeat-gap reproduction matrix
# (docs/heartbeat-gap-repro.md). It mirrors Hawk's slice shape, scaled to the
# guest: cowboy.slice/cowboy-agents.slice carries the task budget (Hawk: 48G
# high / 64G max of 93 GiB; here 2G / 2816M of 4 GiB) and
# cowboy.slice/cowboy-control.slice is the protected sibling a later split
# would use. The harness runs in the system manager; the cgroup v2 limits act
# the same way as under Hawk's user manager. No secret enters this module.
{ pkgs, ... }:
let
  hbrepro = pkgs.writers.writeRustBin "hbrepro" {
    rustcArgs = [
      "--edition=2021"
      "-O"
    ];
  } (builtins.readFile ./hbrepro.rs);
in
{
  system.stateVersion = "25.05";
  services.openssh.enable = true;
  virtualisation = {
    graphics = false;
    memorySize = 4096;
    cores = 4;
    diskSize = 8192;
  };
  zramSwap = {
    enable = true;
    memoryPercent = 50;
  };
  environment.systemPackages = [
    hbrepro
    pkgs.stress-ng
    pkgs.jq
  ];
  environment.etc."hbrepro/matrix.sh".source = ./matrix.sh;
  # matrix.sh refuses to run anywhere this marker is missing.
  environment.etc."hbrepro/guest".text = "heartbeat-gap reproduction guest\n";
  systemd.slices.cowboy-agents.sliceConfig = {
    MemoryAccounting = true;
    MemoryHigh = "2G";
    MemoryMax = "2816M";
    MemorySwapMax = "512M";
  };
  systemd.slices.cowboy-control.sliceConfig = {
    MemoryAccounting = true;
    CPUWeight = 1000;
    IOWeight = 1000;
    MemoryLow = "256M";
  };
}
