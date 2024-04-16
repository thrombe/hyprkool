{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-23.11";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    hyprland = {
      url = "github:hyprwm/Hyprland";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
    };
  };

  outputs = inputs:
    inputs.flake-utils.lib.eachDefaultSystem (system: let
      flakePackage = flake: package: flake.packages."${system}"."${package}";
      flakeDefaultPackage = flake: flakePackage flake "default";

      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [
          (final: prev: {
            unstable = import inputs.nixpkgs-unstable {
              inherit system;
            };
          })
        ];
      };

      manifest = (pkgs.lib.importTOML ./Cargo.toml).package;

      hyprkool-rs = pkgs.unstable.rustPlatform.buildRustPackage {
        pname = manifest.name;
        version = manifest.version;
        cargoLock = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "hyprland-0.3.13" = "sha256-C+mmagn3inZMW+O+0vqTj53z4f8pBxTLbq1Vc341Xjk=";
          };
        };
        src = pkgs.lib.cleanSource ./.;

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];
      };

      fhs = pkgs.buildFHSEnv {
        name = "fhs-shell";
        targetPkgs = p: (env-packages p) ++ custom-commands;
        runScript = "${pkgs.zsh}/bin/zsh";
        profile = ''
          export FHS=1
          # source ./.venv/bin/activate
          # source .env
        '';
      };
      custom-commands = [];

      env-packages = pkgs:
        with pkgs;
          [
            unstable.rust-analyzer
            unstable.rustfmt
            unstable.clippy
            # unstable.rustup

            (pkgs.writeShellScriptBin "kool-meson-configure" ''
              #!/usr/bin/env bash
              rm -rf ./build
              meson setup build --reconfigure
            '')
            (pkgs.writeShellScriptBin "kool-ninja-build" ''
              #!/usr/bin/env bash
              ninja -C build
            '')

            (flakeDefaultPackage inputs.hyprland)
            unstable.clang
            # unstable.gcc
            meson
            ninja
            cmake

            # libdrm
            # pixman
          ]
          ++ (flakeDefaultPackage inputs.hyprland).buildInputs
          ++ hyprkool-rs.nativeBuildInputs;
    in {
      packages.hyprkool-rs = hyprkool-rs;
      packages.default = hyprkool-rs;

      devShells.default =
        pkgs.mkShell.override {
          stdenv = pkgs.clangStdenv;
          # stdenv = pkgs.gccStdenv;
        } {
          nativeBuildInputs = (env-packages pkgs) ++ [fhs];
          shellHook = ''
            export RUST_BACKTRACE="1"

            # $(pwd) always resolves to project root :)
            export CLANGD_FLAGS="--compile-commands-dir=$(pwd)/plugin --query-driver=$(which $CXX)"
          '';
        };
    });
}
