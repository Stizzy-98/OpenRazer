{
  description = "openrazer: Razer laptop control for Linux";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };

  outputs =
    inputs@{
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        name = "openrazer";
      in
      {
        formatter = pkgs.nixfmt-rfc-style;

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = name;
          version = "0.4.0";

          nativeBuildInputs = with pkgs; [
            pkg-config
            wrapGAppsHook4
          ];
          buildInputs = with pkgs; [
            dbus.dev
            hidapi
            systemd
            glib
            gtk4
            libadwaita
          ];

          src = ./.;

          postConfigure = ''
            substituteInPlace src/lib.rs --replace-fail '/usr/share/razercontrol/laptops.json' '${./data/devices/laptops.json}'
          '';

          postInstall = ''
            mkdir -p $out/lib/udev/rules.d
            mkdir -p $out/libexec
            mkdir -p $out/share/applications
            mv $out/bin/daemon $out/libexec
            cp ${./data/udev/70-openrazer-hidraw.rules} $out/lib/udev/rules.d/70-openrazer-hidraw.rules
            cp ${./data/gui/io.github.stizzy98.openrazer.desktop} $out/share/applications/io.github.stizzy98.openrazer.desktop
          '';

          cargoLock = {
            lockFile = ./Cargo.lock;
          };
        };
      }
    )
    // {
      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        with lib;
        let
          cfg = config.services.openrazer;
        in
        {
          options.services.openrazer = {
            enable = mkEnableOption "openrazer (Razer laptop control)";
            package = mkOption {
              type = types.package;
              default = inputs.self.packages.${pkgs.stdenv.hostPlatform.system}.default;
            };
          };

          config = mkIf cfg.enable {
            services.upower.enable = true;
            environment.systemPackages = [ cfg.package ];
            services.udev.packages = [ cfg.package ];

            systemd.user.services."razerdaemon" = {
              description = "Razer laptop control daemon";
              serviceConfig = {
                Type = "simple";
                ExecStartPre = "${pkgs.coreutils}/bin/mkdir -p %h/.local/share/razercontrol";
                ExecStart = "${cfg.package}/libexec/daemon";
              };
              wantedBy = [ "default.target" ];
            };
          };
        };
    };
}
