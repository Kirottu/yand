{
  description = "Description for the project";

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
        };
      flake = {
        homeModules = {
          yand = import ./nix/modules/home-manager.nix self;
        };
      };
    };
}
