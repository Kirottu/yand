{
  description = "Yet Another Notification Daemon";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    home-manager.url = "github:nix-community/home-manager";
  };

  outputs =
    inputs@{ self, flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.home-manager.flakeModules.home-manager
      ];
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      perSystem =
        {
          config,
          self',
          inputs',
          pkgs,
          system,
          ...
        }:
        {
          packages = {
            yand = pkgs.callPackage ./nix/packages/yand.nix { inherit inputs; };
            default = self'.packages.yand;
          };
          devShells = {
            default = pkgs.mkShell {
              inputsFrom = builtins.attrValues self'.packages;
              packages = with pkgs; [
                rustc
                gcc
                gdb
                pkg-config
                cargo
                clippy
                rustfmt
              ];
            };
          };
        };
      flake = {
        homeModules = {
          yand = import ./nix/modules/home-manager.nix self;
        };
      };
    };
}
