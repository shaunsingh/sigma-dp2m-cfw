{
  description = "dp2m-fw: Sigma DP2 Merrill firmware toolchain";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      devShells = forAll (pkgs: {
        # stdenv supplies the host cc and make that mlua needs to build the vendored LuaJIT;
        # the unwrapped clang cross-compiles the M6M payload without host flags
        default = pkgs.mkShell {
          packages = with pkgs; [
            git
            rustc
            cargo
            clippy
            rustfmt
            rust-analyzer
            llvmPackages.clang-unwrapped
            llvmPackages.lld
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
    };
}
