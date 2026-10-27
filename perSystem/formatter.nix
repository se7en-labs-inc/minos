{
  perSystem = {
    config,
    inputs',
    lib,
    ...
  }: {
    treefmt = {
      projectRootFile = "flake.nix";

      programs.alejandra.enable = true;
      programs.rustfmt.enable = true;
      # Match cargo fmt's pinned Fenix toolchain and the workspace's Rust edition.
      settings.formatter.rustfmt.command = lib.mkForce "${inputs'.fenix.packages.stable.rustfmt}/bin/rustfmt";
      settings.formatter.rustfmt.options = lib.mkForce [
        "--config"
        "skip_children=true"
        "--edition"
        "2021"
      ];
      programs.taplo.enable = true;

      programs.elm-format.enable = true;

      settings.global.excludes = [
        "*.lock"
        "*.patch"
        ".gitattributes"
        ".gitignore"
        ".gitmodules"
        "LICENSE"
        "elm-stuff"
      ];
    };

    formatter = config.treefmt.build.wrapper;
  };
}
