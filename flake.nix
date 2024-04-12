{
  description = "yaaaaaaaaaaaaaaaaaaaaa";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-23.11";
    nixpkgs-unstable.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = inputs @ {
    self,
    ...
  }:
    inputs.flake-utils.lib.eachDefaultSystem (system: let
      pkgs = import inputs.nixpkgs {
        inherit system;
      };
      unstable = import inputs.nixpkgs-unstable {
        inherit system;
      };

      manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
    in {
      packages.default = unstable.rustPlatform.buildRustPackage {
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

      devShells.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs;
          [
            unstable.rust-analyzer
            unstable.rustfmt
            unstable.clippy
            # unstable.rustup
          ]
          ++ self.packages."${system}".default.nativeBuildInputs;
        shellHook = ''
          export RUST_BACKTRACE="1"
        '';
      };
    });
}
