{inputs, ...}: {
  perSystem = {
    inputs',
    system,
    config,
    lib,
    pkgs,
    ...
  }: let
    toolchain = with inputs'.fenix.packages;
      combine [
        stable.rustc
        stable.cargo
        stable.clippy
        stable.rustfmt
      ];

    craneLib = (inputs.crane.mkLib pkgs).overrideToolchain toolchain;

    src = lib.fileset.toSource {
      root = ./..;
      fileset = lib.fileset.unions [
        ../Cargo.lock
        ../Cargo.toml
        ../minos/Cargo.toml
        ../minos/src
        ../minos-portal/Cargo.toml
        ../minos-portal/src
      ];
    };

    # Extract pname and version from Cargo.toml
    crateInfo = craneLib.crateNameFromCargoToml {cargoToml = ../minos/Cargo.toml;};

    commonArgs = {
      inherit src;
      inherit (crateInfo) pname version;
      strictDeps = true;

      nativeBuildInputs = with pkgs; [
        cmake
        pkg-config
      ];

      buildInputs = with pkgs; [openssl zlib];

      meta = {
        mainProgram = crateInfo.pname;
        maintainers = with lib.maintainers; [
          disassembler
        ];
        license = with lib.licenses; [
          asl20
        ];
      };
    };

    # Build dependencies separately for better caching
    cargoArtifacts = craneLib.buildDepsOnly commonArgs;

    minosUi = pkgs.stdenv.mkDerivation {
      pname = "minos-ui";
      version = "0.1.0";

      src = lib.fileset.toSource {
        root = ./..;
        fileset = lib.fileset.unions [
          ../minos-ui/elm.json
          ../minos-ui/elm-packages.nix
          ../minos-ui/registry.dat
          ../minos-ui/shell.html
          ../minos-ui/src
        ];
      };

      nativeBuildInputs = with pkgs; [elmPackages.elm];

      configurePhase = pkgs.elmPackages.fetchElmDeps {
        elmPackages = import ../minos-ui/elm-packages.nix;
        elmVersion = "0.19.1";
        registryDat = ../minos-ui/registry.dat;
      };

      buildPhase = ''
        cd minos-ui
        elm make src/Main.elm --optimize --output=main.js
      '';

      installPhase = ''
        mkdir -p $out
        cp main.js $out/main.js
        cp shell.html $out/index.html
      '';

      meta = {
        maintainers = with lib.maintainers; [disassembler];
        license = with lib.licenses; [asl20];
      };
    };
  in {
    checks = {
      clippy = craneLib.cargoClippy (commonArgs
        // {
          inherit cargoArtifacts;
          cargoClippyExtraArgs = "--all-targets -- --deny warnings";
        });

      fmt = craneLib.cargoFmt {inherit src;};

      inherit minosUi;
    };

    packages = {
      default = config.packages.${crateInfo.pname};

      ${crateInfo.pname} = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          doCheck = true;
        });

      minos-ui = minosUi;
    };
  };
}
