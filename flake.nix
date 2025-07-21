{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    hyprland = {
      url = "github:hyprwm/Hyprland/v0.50.0";
    };
    nixpkgs.follows = "hyprland/nixpkgs";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = inputs: let
    packages = inputs.flake-utils.lib.eachDefaultSystem (system: let
      flakePackage = flake: package: flake.packages."${system}"."${package}";
      flakeDefaultPackage = flake: flakePackage flake "default";

      pkgs = import inputs.nixpkgs {
        inherit system;
        overlays = [
          (self: super: {
            hyprland = flakeDefaultPackage inputs.hyprland;
          })
        ];
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
            "hyprland-0.4.0-alpha.3" = "sha256-dUJOOQeh1iBC3W2DWmaHdbs9DnufeZzMOdrrhPFHf70=";
          };
        };
        src = pkgs.lib.cleanSource ./.;

        nativeBuildInputs = with pkgs; [
          pkg-config
        ];

        inherit meta;
      };
      plugin-manifest = (pkgs.lib.importTOML ./hyprpm.toml).repository;
      # - [Override Design Pattern - Nix Pills](https://nixos.org/guides/nix-pills/14-override-design-pattern)
      hyprkool-plugin = pkgs.lib.makeOverridable pkgs.callPackage ({
        pkgs,
        hyprland,
      }:
        stdenv.mkDerivation rec {
          pname = plugin-manifest.name;
          version = manifest.version;

          src = ./.;

          dontUseCmakeConfigure = true;
          dontUseMesonConfigure = true;
          buildPhase = ''
            make -j 16 plugin
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
              hyprland.dev
            ];
          buildInputs =
            (with pkgs; [
              cmake
              meson
              ninja
            ])
            ++ hyprland.buildInputs;

          inherit meta;
        }) {};

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

          # - [NixOS virtual machines — nix.dev documentation](https://nix.dev/tutorials/nixos/nixos-configuration-on-vm.html)
          # - [GitHub - astro/microvm.nix: NixOS MicroVMs](https://github.com/astro/microvm.nix)
          # - [GitHub - nix-community/nixos-generators: Collection of image builders [maintainer=@Lassulus]](https://github.com/nix-community/nixos-generators)
          #   - [nixos-config/flake.nix · Nero-Study-Hat/nixos-config · GitHub](https://github.com/Nero-Study-Hat/nixos-config/blob/64e26b9773a0d38802358d74db691d4eb3f1e91e/flake.nix)

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
          # (flakePackage inputs.hyprland "hyprland")
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
            export CLANGD_FLAGS="--compile-commands-dir=$(pwd)/plugin/build --query-driver=$(which $CXX)"
          '';
        };
    });
    vm = {
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

                  wayland-protocols
                  wayland-utils

                  kitty
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
                qemu.options = [
                  # "-device virtio-vga"

                  "-device virtio-vga-gl"
                  "-display gtk,gl=on"
                  "-enable-kvm"
                ];
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
              hardware.graphics.enable = true;
            })
            ({...}: let
              hyprkool-rs = flakePackage packages "hyprkool-rs";
              hyprkool-plugin = flakePackage packages "hyprkool-plugin";
            in {
              environment.systemPackages =
                (with pkgs; [
                  helix
                ])
                ++ [
                  hyprkool-rs
                  hyprkool-plugin
                  (pkgs.writeShellScriptBin "kool-launch" ''
                    #!/usr/bin/env bash
                    echo "hyprctl plugin load ${hyprkool-plugin}/lib/libhyprkool.so" > ~/load.sh
                    chmod +x ~/load.sh
                    echo "hyprctl plugin unload ${hyprkool-plugin}/lib/libhyprkool.so" > ~/unload.sh
                    chmod +x ~/unload.sh

                    Hyprland
                  '')
                ];
            })
          ];
        };
    };
  in
    inputs.nixpkgs.lib.attrsets.recursiveUpdate packages vm;
}
