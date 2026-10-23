{
  description = "macshell-x: a macOS Tahoe-style shell for niri";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { nixpkgs, ... }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (
        pkgs:
        let
          # Loaded with dlopen at runtime by wgpu and the Wayland client stack.
          runtimeLibs = with pkgs; [
            wayland
            libxkbcommon
            vulkan-loader
            libGL
          ];
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              clippy
              rustfmt
              rust-analyzer
              pkg-config
            ];
            buildInputs = runtimeLibs;
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
          };
        }
      );
    };
}
