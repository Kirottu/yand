self:
{
  config,
  pkgs,
  lib,
  ...
}:
let
  inherit (lib.options) mkOption mkEnableOption;
  inherit (lib.types) nullOr package;
in
{
  meta.maintainers = with lib.maintainers; [ Kirottu ];

  options.programs.anyrun = {
    enable = mkEnableOption "anyrun";
    package = mkOption {
      type = nullOr package;
      default = self.packages.${pkgs.system}.yand;
    };
  };
}
