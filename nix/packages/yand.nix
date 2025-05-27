{
  inputs,
  lib,
  glib,
  rustPlatform,
  gtk4,
  gtk4-layer-shell,
  pkg-config,
  cargo,
  rustc,
  ...
}:
let
  cargoToml = builtins.fromTOML (builtins.readFile ../../Cargo.toml);
  pname = cargoToml.package.name;
  version = cargoToml.package.version;
in
rustPlatform.buildRustPackage {
  inherit pname version;
  src = builtins.path {
    path = lib.sources.cleanSource inputs.self;
    name = "${pname}-${version}";
  };

  strictDeps = true;

  cargoLock = ../../Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    rustc
    cargo
  ];

  buildInputs = [
    glib
    gtk4
    gtk4-layer-shell
  ];

  doCheck = true;
  checkInputs = [
    cargo
    rustc
  ];

  CARGO_BUILD_INCREMENTAL = "false";
  RUST_BACKTRACE = "full";

  meta = {
    description = "Yet Another Notification Daemon";
    homepage = "https://github.com/Kirottu/yand";
    mainProgram = "yand";
    maintainers = with lib.maintainers; [ Kirottu ];
  };
}
