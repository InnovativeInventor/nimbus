{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    nativeBuildInputs = with pkgs; [
      cargo
      gcc_latest

      sccache
      mold

      pkg-config
      fuse3

      busybox
    ];
}
