{
  perSystem = {
    config,
    pkgs,
    ...
  }: {
    treefmt = {
      projectRootFile = "flake.nix";

      programs.alejandra.enable = true;
      programs.rustfmt.enable = true;
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
