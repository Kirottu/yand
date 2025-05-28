self:
{
  config,
  pkgs,
  lib,
  ...
}:
let
  inherit (lib)
    mkIf
    getExe
    literalExpression
    isStorePath
    ;
  inherit (lib.options) mkOption mkEnableOption;
  inherit (lib.types)
    nullOr
    either
    path
    lines
    package
    ;

  cfg = config.services.yand;
  tomlFormat = pkgs.formats.toml { };
in
{
  meta.maintainers = with lib.maintainers; [ Kirottu ];

  options.services.yand = {
    enable = mkEnableOption "yand";
    package = mkOption {
      type = nullOr package;
      default = self.packages.${pkgs.system}.yand;
    };
    settings = mkOption {
      type = tomlFormat.type;
      default = { };
      example = literalExpression ''
        {
          width = 400;
          spacing = 10;
          output = "eDP-1";
          timeout = 10;
        };
      '';
      description = ''
        Configuration file written to {file}`$XDG_CONFIG_HOME/yand/config.toml`

        See <https://github.com/Kirottu/yand> for all options.
      '';
    };
    style = mkOption {
      type = nullOr (either path lines);
      default = null;
      description = ''
        CSS style for the notifications.
      '';
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      (lib.hm.assertions.assertPlatform "services.yand" pkgs lib.platforms.linux)
    ];

    home.packages = [ cfg.package ];
    dbus.packages = [ cfg.package ];

    systemd.user.services.yand = mkIf (cfg.package != null) {
      Unit = {
        Description = "Yet Another Notification Daemon";
        PartOf = [ config.wayland.systemd.target ];
        After = [ config.wayland.systemd.target ];
        ConditionEnvironment = "WAYLAND_DISPLAY";
      };
      Service = {
        Type = "dbus";
        BusName = "org.freedesktop.Notifications";
        ExecStart = "${getExe cfg.package}";
        Restart = "on-failure";
      };
      Install.WantedBy = [ config.wayland.systemd.target ];
    };

    xdg.configFile."yand/config.toml" = mkIf (cfg.settings != { }) {
      onChange = "${cfg.package}/bin/yandctl reload";
      source = tomlFormat.generate "yand-config" cfg.settings;
    };
    xdg.configFile."yand/style.css" = mkIf (cfg.style != null) {
      onChange = "${cfg.package}/bin/yandctl reload";
      source =
        if builtins.isPath cfg.style || isStorePath cfg.style then
          cfg.style
        else
          pkgs.writeText "yand/style.css" cfg.style;
    };
  };
}
