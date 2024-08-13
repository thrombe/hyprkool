{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    hyprland = {
      # - [submodules still not in nix latest](https://github.com/NixOS/nix/pull/7862#issuecomment-1908577578)
      url = "https://github.com/hyprwm/Hyprland?ref=refs/tags/v0.42.0";
      inputs.nixpkgs.follows = "nixpkgs";
      type = "git";
      submodules = true;
    };
  };

  outputs = inputs:
    inputs.nixpkgs.lib.attrsets.recursiveUpdate (inputs.flake-utils.lib.eachDefaultSystem (system: let
      flakePackage = flake: package: flake.packages."${system}"."${package}";
      flakeDefaultPackage = flake: flakePackage flake "default";

      pkgs = import inputs.nixpkgs {
        inherit system;
      };

      # - [Get-flake: builtins.getFlake without the restrictions - Announcements - NixOS Discourse](https://discourse.nixos.org/t/get-flake-builtins-getflake-without-the-restrictions/17662)
      # hyprland-flake = import "${inputs.hyprland}/flake.nix";
      # hyprland-outputs = hyprland-flake.outputs {
      #   nixpkgs = inputs.nixpkgs-unstable;
      # };
      # hyprland-outputs = inputs.hyprland;

      meta = {
        homepage = manifest.repository;
        description = manifest.description;
        license = pkgs.lib.licenses.mit;
        platforms = pkgs.lib.platforms.linux;
      };
      manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
      hyprkool-rs = pkgs.rustPlatform.buildRustPackage {
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

        nativeBuildInputs =
          (with pkgs; [
            pkg-config
          ])
          ++ [
            (flakeDefaultPackage inputs.hyprland).dev
          ];
        buildInputs =
          (with pkgs; [
            cmake
            meson
            ninja
          ])
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
        (pkgs.writeShellScriptBin "build-vm" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT

          nixos-rebuild build-vm --flake .#test
        '')
        (pkgs.writeShellScriptBin "run-vm" ''
          #!/usr/bin/env bash
          cd $PROJECT_ROOT

          ./result/bin/run-kool-vm $@
        '')
      ];

      env-packages = pkgs:
        (with pkgs; [
          rust-analyzer
          rustfmt
          clippy
          # rustup
        ])
        ++ [
          (flakePackage inputs.hyprland "hyprland-debug")
        ]
        ++ (custom-commands pkgs);

      stdenv = pkgs.clangStdenv;
      # stdenv = pkgs.gccStdenv;
      # stdenv = pkgs.gcc13Stdenv;
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
    })) {
      # nixos-rebuild build-vm --flake .#test
      nixosConfigurations.test = let
        system = "x86_64-linux";
        username = "kool";
        flakePackage = flake: package: flake.packages."${system}"."${package}";
        flakeDefaultPackage = flake: flakePackage flake "default";

        pkgs = import inputs.nixpkgs {
          inherit system;
        };
        hyprland = flakeDefaultPackage inputs.hyprland;
      in
        # https://discourse.nixos.org/t/eval-config-returning-called-with-unexpected-argument-when-running-nixos-rebuild/24960/2
        inputs.nixpkgs.lib.nixosSystem {
          inherit system;
          specialArgs = {inherit inputs username;};
          modules = [
            # - [Installing NixOS with Hyprland! - by Josiah - Brown Noise](https://josiahalenbrown.substack.com/p/installing-nixos-with-hyprland)
            ({...}: {
              networking.hostName = username;
              networking.networkmanager.enable = true;
              users.users."${username}" = {
                isNormalUser = true;
                extraGroups = [
                  "networkmanager"
                  "wheel" # enable sudo for this user
                ];
                group = username;
                # - [NixOS:nixos-rebuild build-vm](https://nixos.wiki/wiki/NixOS:nixos-rebuild_build-vm)
                initialPassword = "${username}";
                # password = "";
              };
              security.sudo = {
                enable = true;
                wheelNeedsPassword = false;
              };

              users.groups."${username}" = {};

              programs.hyprland = {
                enable = true;
                package = hyprland;
              };
              environment.sessionVariables = {
                WLR_RENDERER_ALLOW_SOFTWARE = "1";
              };

              environment.systemPackages =
                (with pkgs; [
                  swww
                  xdg-desktop-portal-gtk
                  xdg-desktop-portal-hyprland
                  # xwayland

                  meson
                  wayland-protocols
                  wayland-utils
                  wl-clipboard
                  # wlroots

                  mesa
                  mesa_drivers

                  kitty
                  glxinfo
                ])
                ++ [
                  hyprland
                ];

              system.stateVersion = "23.05";
            })
            ({modulesPath, ...}: {
              services.spice-vdagentd.enable = true;
              services.qemuGuest.enable = true;

              boot.kernelModules = ["drm" "virtio_gpu"];

              imports = [
                (modulesPath + "/virtualisation/qemu-vm.nix")
              ];

              virtualisation = {
                virtualbox.guest.enable = true;
                vmware.guest.enable = true;
                qemu.options = ["-device virtio-vga"];
              };
              virtualisation.vmVariant = {
                # - [nixpkgs/nixos/modules/virtualisation/qemu-vm.nix at nixos-23.05 · NixOS/nixpkgs · GitHub](https://github.com/NixOS/nixpkgs/blob/nixos-23.05/nixos/modules/virtualisation/qemu-vm.nix)
                virtualisation = {
                  memorySize = 2048;
                  cores = 2;
                  sharedDirectories = {
                    project_dir = {
                      source = builtins.toString ./.;
                      target = "/mnt/shared";
                    };
                  };
                };
              };
              hardware.opengl.enable = true;
            })
            ({...}: {
              environment.pathsToLink = ["/libexec"]; # links /libexec from derivations to /run/current-system/sw
              services.dbus.enable = true;
              xdg.portal = {
                enable = true;
                wlr.enable = true;
                extraPortals = [
                  pkgs.xdg-desktop-portal-gtk
                ];
              };
              programs.sway.enable = true;
              # services.xserver = {
              #   enable = true;
              #   displayManager.gdm.enable = true;
              #   # - [Adding qemu-guest-agent to a nixos VM](https://discourse.nixos.org/t/adding-qemu-guest-agent-to-a-nixos-vm/5931)
              #   videoDrivers = ["qxl" "cirrus" "vmware" "vesa" "modesetting"];
              # };

              environment.systemPackages = with pkgs; [
                helix
                glfw-wayland
                glfw
              ];
            })
          ];
        };
    };
}
