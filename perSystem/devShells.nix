{
  perSystem = {
    config,
    pkgs,
    inputs',
    ...
  }: let
    toolchain = with inputs'.fenix.packages;
      combine [
        stable.rustc
        stable.cargo
        stable.clippy
        stable.rustfmt
        stable.rust-analyzer
      ];
  in {
    devShells.default = with pkgs;
      mkShell {
        packages = [
          toolchain
          cmake
          pkg-config
          openssl
          gnupg
          zlib
          config.treefmt.build.wrapper
          # Elm
          elmPackages.elm
          elmPackages.elm-format
          elmPackages.elm-language-server
          elmPackages.elm-test
          elmPackages.elm-live
          elm2nix
        ];
        LD_LIBRARY_PATH = lib.makeLibraryPath [stdenv.cc.cc.lib];
      };
  };
}
