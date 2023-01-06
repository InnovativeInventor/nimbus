{ pkgs ? import <nixpkgs> {} }:
  pkgs.mkShell {
    nativeBuildInputs = with pkgs; [
      cargo
      gcc_latest
      gdb

      sccache
      mold

      pkg-config
      fuse3

      busybox
      tree
    ];
}
