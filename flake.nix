{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    hyprland = {
      # url = "github:hyprwm/Hyprland/v0.41.2";
      # url = "https://github.com/hyprwm/Hyprland?ref=v0.41.2";
      url = "https://github.com/hyprwm/Hyprland?ref=refs/tags/v0.41.2";
      # - [submodules still not in nix latest](https://github.com/NixOS/nix/pull/7862#issuecomment-1908577578)
      # url = "git+https://github.com/hyprwm/Hyprland/?rev=2b520571e897be2a0e88c8692da607b062000038&submodules=1"; # 0.41.2
      inputs.nixpkgs.follows = "nixpkgs-unstable";
      type = "git";
      submodules = true;
      # flake = false;
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

      # - [Get-flake: builtins.getFlake without the restrictions - Announcements - NixOS Discourse](https://discourse.nixos.org/t/get-flake-builtins-getflake-without-the-restrictions/17662)
      # hyprland-flake = import "${inputs.hyprland}/flake.nix";
      # hyprland-outputs = hyprland-flake.outputs {
      #   nixpkgs = inputs.nixpkgs-unstable;
      # };
      # hyprland-outputs = inputs.hyprland;

      meta = with pkgs.lib; {
        homepage = manifest.repository;
        description = manifest.description;
        license = licenses.mit;
        platforms = platforms.linux;
      };
      manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
      hyprkool-rs = pkgs.unstable.rustPlatform.buildRustPackage {
        pname = manifest.name;
        version = manifest.version;
        cargoLock = {
          lockFile = ./Cargo.lock;
          outputHashes = {
            "hyprland-0.4.0-alpha.2" = "sha256-7GRj0vxsQ4ORp0hSBAorjFYvWDy+edGU2IL3DhFDLvQ=";
          };
        };
        src = pkgs.lib.cleanSource ./.;

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        inherit meta;
      };
      plugin-manifest = (pkgs.lib.importTOML ./hyprpm.toml).repository;
      hyprkool-plugin = stdenv.mkDerivation rec {
        pname = plugin-manifest.name;
        version = manifest.version;

        src = ./.;

        dontUseCmakeConfigure = true;
        dontUseMesonConfigure = true;
        buildPhase = ''
          make plugin
          mv ./plugin/build/lib${pname}.so .
        '';
        installPhase = ''
          mkdir -p $out/lib
          mv ./lib${pname}.so $out/lib/lib${pname}.so
        '';

        nativeBuildInputs = with pkgs; [
          pkg-config
          (flakeDefaultPackage inputs.hyprland).dev
        ];
        buildInputs = with pkgs;
          [
            cmake
            meson
            ninja
          ]
          ++ (flakeDefaultPackage inputs.hyprland).buildInputs;

        inherit meta;
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

          make plugin-meson-configure
        '')
        (pkgs.writeShellScriptBin "kool-ninja-build" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT

          make plugin-ninja-build
        '')
        (pkgs.writeShellScriptBin "kool-cmake-build" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT

          make plugin-cmake-build
        '')
        (pkgs.writeShellScriptBin "kool-test" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT
          ctrl_c_handler() {
            echo "Ctrl+C pressed, stopping Hyprland..."
            kill "$hyprland_pid"
            exit 0
          }
          trap ctrl_c_handler INT

          Hyprland
          # ~/1Git/code_read/Hyprland/result/bin/Hyprland
          # hyprland_pid=$!

          # sleep 5

          # instance="$(hyprctl instances -j | jq -r '. | length - 1')"
          # hyprctl -i $instance plugin load $(realpath ./plugin/build/libhyprkool.so)

          # wait $hyprland_pid
        '')
        (pkgs.writeShellScriptBin "kool-reload" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT
          instance="$(hyprctl instances -j | jq -r '. | length - 1')"
          hyprctl -i $instance plugin unload $(realpath ./plugin/build/libhyprkool.so)
          hyprctl -i $instance plugin load $(realpath ./plugin/build/libhyprkool.so)
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
          ]
          ++ (custom-commands pkgs);

      stdenv = pkgs.unstable.clangStdenv;
      # stdenv = pkgs.unstable.gccStdenv;
      # stdenv = pkgs.unstable.gcc13Stdenv;
    in {
      packages = {
        default = hyprkool-rs;
        inherit hyprkool-rs hyprkool-plugin;
      };

      devShells.default =
        pkgs.mkShell.override {
          inherit stdenv;
        } {
          nativeBuildInputs = (env-packages pkgs) ++ [fhs];
          inputsFrom = [
            hyprkool-rs
            hyprkool-plugin
          ];
          shellHook = ''
            export PROJECT_ROOT="$(pwd)"

            export RUST_BACKTRACE="1"

            # $(pwd) always resolves to project root :)
            export CLANGD_FLAGS="--compile-commands-dir=$(pwd)/plugin --query-driver=$(which $CXX)"
          '';
        };
    });
}
