# Yand

Yet Another Notification Daemon, a simple notification daemon with GTK CSS styling
and integrated notification action support.

## Installation

Build the project in release mode with `cargo build --release` and install the
built binaries into appropriate locations.

### NixOS

A Nix flake is a part of the repository. Add the following to your inputs:

```nix
yand = {
  url = "github:Kirottu/yand";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

And then the following to your Home Manager config:

```nix
imports = [
  inputs.yand.homeModules.yand
];

services.yand = {
  enable = true;
  settings = {
    # Settings here
  };
  style = ''''; # Either a path to a CSS file or a string containing the CSS styling
}
```

## Configuration

Yand uses a TOML config file placed in `.config/yand/config.toml`. The supported config options
are explained in the following config snippet:

```toml
# The output that notifications will be shown on. If unavailable another available one
# (most likely currently focused output) will be used
output = "DP-3"
# Layer shell layer that the notifications are placed on. Available values:
# background, bottom, top, overlay
layer = "overlay"
# Spacing between notifications
spacing = 10
# The default timeout. Can be overridden by applications or the config
timeout = 10
# The width of the notifications
width = 400
# Maximum amount of text lines allowed in the notification. The rest is truncated.
max_lines = 5
# The size of the icon if provided by the application
icon_size = 64

[[app_override]]
# Name of the application as provided by the application
# Check Yand logs to figure out what applications provide
#
# Must be provided for the app override
app_name = "discord"
# Overridden timeout per application, will override the default
# config and anything requested by the application
timeout = 5
# Override default max_lines
max_lines = 10
```

## Feedback

Any feedback on anything this project related is appreciated, it currently supports a set of features
used by me and thus may miss features used by other people.
