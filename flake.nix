{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.05";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";

    hyprland = {
      # - [submodules still not in nix latest](https://github.com/NixOS/nix/pull/7862#issuecomment-1908577578)
      url = "https://github.com/hyprwm/Hyprland?ref=refs/tags/v0.42.0";
      inputs.nixpkgs.follows = "nixpkgs-unstable";
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
    })) {
      nixosConfigurations.test =
        # nixos-rebuild build-vm --flake .#test
        let
          system = "x86_64-linux";
          username = "kool";
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
          hyprland = flakeDefaultPackage inputs.hyprland;
        in
          # https://discourse.nixos.org/t/eval-config-returning-called-with-unexpected-argument-when-running-nixos-rebuild/24960/2
          inputs.nixpkgs.lib.nixosSystem {
            inherit system;
            specialArgs = {inherit inputs username;};
            modules = [
              # ({
              #   config,
              #   lib,
              #   # pkgs,
              #   modulesPath,
              #   ...
              # }: {
              #   imports = [
              #     (modulesPath + "/installer/scan/not-detected.nix")
              #   ];
              #   boot.initrd.availableKernelModules = ["xhci_pci" "ahci" "nvme" "usbhid" "sd_mod" "sr_mod" "rtsx_pci_sdmmc"];
              #   boot.initrd.kernelModules = [];
              #   boot.kernelModules = ["kvm-intel"];
              #   boot.extraModulePackages = [];

              #   # fileSystems."/" = {
              #   #   device = "/dev/disk/by-uuid/ad0ef280-0c6d-40fc-a5b6-6fe14b547bd2";
              #   #   fsType = "ext4";
              #   # };

              #   # fileSystems."/boot" = {
              #   #   device = "/dev/disk/by-uuid/68D7-B0A1";
              #   #   fsType = "vfat";
              #   # };

              #   swapDevices = [];

              #   # Enables DHCP on each ethernet and wireless interface. In case of scripted networking
              #   # (the default) this is the recommended approach. When using systemd-networkd it's
              #   # still possible to use this option, but it's recommended to use it in conjunction
              #   # with explicit per-interface declarations with `networking.interfaces.<interface>.useDHCP`.
              #   networking.useDHCP = lib.mkDefault true;
              #   # networking.interfaces.enp3s0.useDHCP = lib.mkDefault true;
              #   # networking.interfaces.wlp2s0.useDHCP = lib.mkDefault true;

              #   nixpkgs.hostPlatform = lib.mkDefault "x86_64-linux";
              #   powerManagement.cpuFreqGovernor = lib.mkDefault "powersave";
              #   hardware.cpu.intel.updateMicrocode = lib.mkDefault config.hardware.enableRedistributableFirmware;
              # })

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
                  # WAYLAND_DISPLAY = "wayland-1";
                  # DISPLAY = ":0";
                  # SDL_VIDEODRIVER = "wayland";
                  # CLUTTER_BACKEND = "wayland";
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
                    wlroots

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
              ({ modulesPath, ...}: {
                environment.pathsToLink = [ "/libexec" ]; # links /libexec from derivations to /run/current-system/sw 
                services.spice-vdagentd.enable = true;
                services.qemuGuest.enable = true;

                boot.kernelModules = [ "drm" "virtio_gpu" ];

                imports = [
                  (modulesPath + "/virtualisation/qemu-vm.nix")
                ];

                virtualisation = {
                  virtualbox.guest.enable = true;
                  vmware.guest.enable = true;
                  qemu.options = [
                    "-device virtio-vga"
                    # "--virtio-gpu"
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
                hardware.opengl.enable = true;
              })
              ({...}: {
                services.dbus.enable = true;
                xdg.portal = {
                  enable = true;
                  wlr.enable = true;
                  extraPortals = [
                    pkgs.xdg-desktop-portal-gtk
                  ];
                };
                programs.sway.enable = true;
                # services.xserver.enable = true;
                # services.xserver.displayManager.gdm.enable = true;

                environment.systemPackages = with pkgs; [
                  helix
                  glfw-wayland
                  glfw
                ];
              })
              # ({...}: {
              #   # Use the systemd-boot EFI boot loader.
              #   boot.loader.systemd-boot.enable = true;
              #   boot.loader.efi.canTouchEfiVariables = true;

              #   networking.hostName = "nixos"; # Define your hostname.
              #   # Pick only one of the below networking options.
              #   # networking.wireless.enable = true;  # Enables wireless support via wpa_supplicant.
              #   networking.networkmanager.enable = true; # Easiest to use and most distros use this by default.

              #   # Set your time zone.
              #   # time.timeZone = "Europe/Amsterdam";

              #   # Select internationalisation properties.
              #   # i18n.defaultLocale = "en_US.UTF-8";
              #   # console = {
              #   #   font = "Lat2-Terminus16";
              #   #   keyMap = "us";
              #   #   useXkbConfig = true; # use xkbOptions in tty.
              #   # };

              #   environment.pathsToLink = ["/libexec"]; # links /libexec from derivations to /run/current-system/sw
              #   services.spice-vdagentd.enable = true;
              #   services.qemuGuest.enable = true;
              #   services.xserver = {
              #     enable = true;

              #     desktopManager = {
              #       xterm.enable = true;
              #     };

              #     displayManager = {
              #       # defaultSession = "none+${hyprland.name}";
              #       # autoLogin = {
              #       #   user = "${wm.name}";
              #       #   enable = true;
              #       # };
              #     };

              #     # qemuGuest.enable = true;
              #     # - [Adding qemu-guest-agent to a nixos VM](https://discourse.nixos.org/t/adding-qemu-guest-agent-to-a-nixos-vm/5931)
              #     videoDrivers = ["qxl" "cirrus" "vmware" "vesa" "modesetting"];

              #     windowManager.session = [
              #       {
              #         name = hyprland.name;
              #         start = ''
              #           mkdir -p ~/.config

              #           # rm -rf ~/.config/picom
              #           # ln -s /mnt/shared/target/picom ~/.config/picom
              #           # ${pkgs.picom}/bin/picom &

              #           rm -rf ~/.config/alacritty
              #           ln -s /mnt/shared/target/alacritty  ~/.config/alacritty

              #           wm_bin="/mnt/shared/target/debug/${hyprland.name}"
              #           wm_log="/mnt/shared/target/log.log"
              #           wm_prev_log="/mnt/shared/target/prev.log"
              #           stat=${pkgs.coreutils}/bin/stat

              #           wm_command() {
              #             mv "$wm_log" "$wm_prev_log"
              #             "$wm_bin" &> "$wm_log"
              #           }

              #           is_command_running() {
              #               pgrep -f "$wm_bin" > /dev/null
              #           }

              #           wm_command

              #           last_modified=$(stat -c %Y "$wm_bin")

              #           while true; do
              #             sleep 1

              #             if ! is_command_running; then
              #               wm_command
              #               continue
              #             fi

              #             current_modified=$(stat -c %Y "$wm_bin")

              #             if [ $last_modified -ne $current_modified ]; then
              #               echo "restarting"
              #               pkill "${hyprland.name}"
              #               wm_command
              #               last_modified="$current_modified"
              #             fi
              #           done
              #         '';
              #       }
              #     ];
              #   };

              #   virtualisation = {
              #     virtualbox.guest.enable = true;
              #     vmware.guest.enable = true;
              #   };
              #   virtualisation.vmVariant = {
              #     # - [nixpkgs/nixos/modules/virtualisation/qemu-vm.nix at nixos-23.05 · NixOS/nixpkgs · GitHub](https://github.com/NixOS/nixpkgs/blob/nixos-23.05/nixos/modules/virtualisation/qemu-vm.nix)
              #     virtualisation = {
              #       # qemu.guestAgent.enable = true;
              #       # virtualisation.vmware.guest.enable = true;
              #       # virtualisation.virtualbox.guest.enable = true;
              #       memorySize = 2048;
              #       cores = 2;
              #       sharedDirectories = {
              #         project_dir = {
              #           source = builtins.toString hyprland.src;
              #           target = "/mnt/shared";
              #         };
              #       };
              #     };
              #   };
              #   # environment.variables = {
              #   #   # NOTE: does not work. this env var should be set up in the host. not in the vm :P
              #   #   # - https://github.com/NixOS/nixpkgs/issues/59219#issuecomment-481571469
              #   #   # QEMU_OPTS = "-enable-kvm -display sdl";
              #   #   # QEMU_OPTS = "-enable-kvm -display sdl -virtfs local,path=/home/issac/0Git/wmmw,mount_tag=host0,security_model=passthrough,id=host0";
              #   # };

              #   # Configure keymap in X11
              #   services.xserver.layout = "us";

              #   # Enable sound.
              #   sound.enable = true;
              #   hardware.pulseaudio.enable = true;

              #   # Enable touchpad support (enabled default in most desktopManager).
              #   services.xserver.libinput.enable = true;

              #   users.users."${hyprland.name}" = {
              #     isNormalUser = true;
              #     # - [NixOS:nixos-rebuild build-vm](https://nixos.wiki/wiki/NixOS:nixos-rebuild_build-vm)
              #     initialPassword = "${hyprland.name}";
              #     extraGroups = ["wheel"]; # Enable ‘sudo’ for the user.
              #     # packages = with pkgs; [];
              #   };

              #   programs.hyprland = {
              #     enable = true;
              #     xwayland.enable = true;
              #   };

              #   environment.systemPackages = with pkgs; [
              #     alacritty
              #     polybarFull
              #     dmenu-rs
              #     helix
              #     git
              #     rofi
              #     feh
              #     zsh
              #     bluez
              #     dunst
              #     lf
              #     du-dust
              #     file
              #     patchelf
              #     vim
              #     wget
              #     mount

              #     xorg.xmodmap

              #     glxinfo
              #   ];

              #   system.stateVersion = "23.05";
              # })
            ];
          };
    };
}
