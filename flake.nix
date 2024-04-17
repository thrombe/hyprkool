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
        targetPkgs = p: (env-packages p) ++ (custom-commands p);
        runScript = "${pkgs.zsh}/bin/zsh";
        profile = ''
          export FHS=1
          # source ./.venv/bin/activate
          # source .env
        '';
      };
      custom-commands = pkgs: [
        (pkgs.writeShellScriptBin "kool-meson-configure" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT
          cd plugin

          rm -rf ./build
          meson setup build --reconfigure
        '')
        (pkgs.writeShellScriptBin "kool-ninja-build" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT
          cd plugin

          ninja -C build
        '')
        (pkgs.writeShellScriptBin "kool-cmake-build" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT
          cd plugin

          mkdir -p build

          cd build
          cmake ..
          make
        '')
        (pkgs.writeShellScriptBin "kool-test" ''
          #!/usr/bin/env bash
          ctrl_c_handler() {
            echo "Ctrl+C pressed, stopping Hyprland..."
            kill "$hyprland_pid"
            exit 0
          }
          trap ctrl_c_handler INT

          Hyprland &
          hyprland_pid=$!

          # sleep 5

          # instance="$(hyprctl instances -j | jq -r '. | length - 1')"
          # hyprctl -i $instance plugin load $(realpath ./plugin/build/hyprkool.so)

          wait $hyprland_pid
        '')
        (pkgs.writeShellScriptBin "kool-reload" ''
          #!/usr/bin/env bash
          instance="$(hyprctl instances -j | jq -r '. | length - 1')"
          hyprctl -i $instance plugin unload $(realpath ./plugin/build/hyprkool.so)
          hyprctl -i $instance plugin load $(realpath ./plugin/build/hyprkool.so)
        '')
        (pkgs.writeShellScriptBin "kool-rebuild-reload" ''
          #!/usr/bin/env bash
          kool-cmake-build
          kool-reload
        '')
      ];

      env-packages = pkgs:
        with pkgs;
          [
            unstable.rust-analyzer
            unstable.rustfmt
            unstable.clippy
            # unstable.rustup

            (flakePackage inputs.hyprland "hyprland-debug")
            unstable.clang
            # unstable.gcc
            meson
            ninja
            cmake

            # libdrm
            # pixman
          ]
          ++ (custom-commands pkgs)
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
            export PROJECT_ROOT="$(pwd)"

            export RUST_BACKTRACE="1"

            # $(pwd) always resolves to project root :)
            export CLANGD_FLAGS="--compile-commands-dir=$(pwd)/plugin --query-driver=$(which $CXX)"
          '';
        };
    });
}
