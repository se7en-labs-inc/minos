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
        ../LICENSE
        ../NOTICE
        ../THIRD_PARTY_NOTICES.md
        ../licenses
        ../README.md
        ../CONTRIBUTING.md
        ../SECURITY.md
        ../CHANGELOG.md
        ../docs/contracts.md
        ../docs/operations.md
        ../docs/support-matrix.md
        ../docs/releasing.md
        ../docs/preview-rehearsal.md
        ../docs/dependencies.md
        ../specs/001-open-source-release
        ../specs/portal-airgap-completion/airgap-quickstart.md
        ../minos-portal/minos-portal.example.toml
        ../minos/Cargo.toml
        ../minos/src
        ../minos/tests
        ../minos-portal/Cargo.toml
        ../minos-portal/src
        (lib.fileset.maybeMissing ../minos-portal/tests)
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
      nativeCheckInputs = with pkgs; [gnupg openssl];

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
          ../LICENSE
          ../NOTICE
          ../licenses
          ../minos-ui/elm-packages.nix
          ../minos-ui/registry.dat
          ../minos-ui/shell.html
          ../minos-ui/config.js
          ../minos-ui/src
          (lib.fileset.maybeMissing ../minos-ui/tests)
        ];
      };

      nativeBuildInputs = with pkgs; [elmPackages.elm nodejs];
      doCheck = true;

      configurePhase = pkgs.elmPackages.fetchElmDeps {
        elmPackages = import ../minos-ui/elm-packages.nix;
        elmVersion = "0.19.1";
        registryDat = ../minos-ui/registry.dat;
      };

      buildPhase = ''
        cd minos-ui
        elm make src/Main.elm --optimize --output=main.js
      '';

      checkPhase = ''
        elm make tests/Tests.elm --output=tests.js
        node tests/run.cjs
      '';

      installPhase = ''
        mkdir -p $out
        cp main.js $out/main.js
        cp shell.html $out/index.html
        cp config.js $out/config.js
        install -Dm644 "$src/LICENSE" "$out/share/doc/minos-ui/LICENSE"
        install -Dm644 "$src/NOTICE" "$out/share/doc/minos-ui/NOTICE"
        for elm_license in browser-1.0.2 core-1.0.5 html-1.0.0 http-2.0.0 json-1.1.3 time-1.0.0 bytes-1.0.8 file-1.0.5 url-1.0.0 virtual-dom-1.0.3; do
          install -Dm644 "$src/licenses/elm-$elm_license.txt" "$out/share/doc/minos-ui/licenses/elm-$elm_license.txt"
        done
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
          # DuckDB links the portal to the C++ runtime; tests and installed
          # executables need it without the development shell's LD_LIBRARY_PATH.
          buildInputs = commonArgs.buildInputs ++ [pkgs.stdenv.cc.cc.lib];
          env.NIX_LDFLAGS = "-rpath ${lib.makeLibraryPath [pkgs.stdenv.cc.cc.lib]}";
          nativeBuildInputs = commonArgs.nativeBuildInputs ++ [pkgs.makeWrapper];
          postInstall = ''
            install -Dm644 "$src/LICENSE" "$out/share/doc/minos/LICENSE"
            install -Dm644 "$src/NOTICE" "$out/share/doc/minos/NOTICE"
            install -Dm644 "$src/THIRD_PARTY_NOTICES.md" "$out/share/doc/minos/THIRD_PARTY_NOTICES.md"
            for license_file in GPL-3.0.txt LGPL-2.1.txt LGPL-3.0.txt GCC-exception-3.1.html duckdb-1.5.5.txt openssl-3.6.4.txt zlib-1.3.2.txt native-runtime.json SHA256SUMS README.md; do
              install -Dm644 "$src/licenses/$license_file" "$out/share/doc/minos/licenses/$license_file"
            done
            for elm_license in browser-1.0.2 core-1.0.5 html-1.0.0 http-2.0.0 json-1.1.3 time-1.0.0 bytes-1.0.8 file-1.0.5 url-1.0.0 virtual-dom-1.0.3; do
              install -Dm644 "$src/licenses/elm-$elm_license.txt" "$out/share/doc/minos/licenses/elm-$elm_license.txt"
            done
            install -Dm644 "$src/licenses/duckdb-1.5.5/SOURCES" "$out/share/doc/minos/licenses/duckdb-1.5.5/SOURCES"
            while IFS= read -r license_path; do
              install -Dm644 "$src/licenses/duckdb-1.5.5/$license_path" "$out/share/doc/minos/licenses/duckdb-1.5.5/$license_path"
            done < "$src/licenses/duckdb-1.5.5/SOURCES"
            for document in \
              CONTRIBUTING.md SECURITY.md CHANGELOG.md \
              docs/contracts.md docs/operations.md docs/support-matrix.md \
              docs/releasing.md docs/preview-rehearsal.md \
              docs/dependencies.md \
              specs/portal-airgap-completion/airgap-quickstart.md \
              specs/001-open-source-release/spec.md \
              specs/001-open-source-release/plan.md \
              specs/001-open-source-release/tasks.md \
              specs/001-open-source-release/research.md \
              specs/001-open-source-release/data-model.md \
              specs/001-open-source-release/quickstart.md \
              specs/001-open-source-release/history-review.md \
              specs/001-open-source-release/dependency-review.md \
              specs/001-open-source-release/contracts/interfaces.md \
              specs/001-open-source-release/checklists/release.md \
              specs/001-open-source-release/checklists/requirements.md; do
              install -Dm644 "$src/$document" "$out/share/doc/minos/$document"
            done
            install -Dm644 "$src/README.md" "$out/share/doc/minos/README.md"
            install -Dm644 "$src/minos-portal/minos-portal.example.toml" "$out/share/doc/minos/minos-portal.example.toml"
            wrapProgram "$out/bin/minos" \
              --prefix PATH : ${lib.makeBinPath [pkgs.gnupg]}
          '';
        });

      minos-ui = minosUi;
    };
  };
}
