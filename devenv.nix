{
  pkgs,
  lib,
  ...
}:

{
  packages = with pkgs; [
    wayland
    libxkbcommon
    pkg-config
    vulkan-loader
  ];
  env.LD_LIBRARY_PATH =
    with pkgs;
    lib.makeLibraryPath [
      libxkbcommon
      wayland
      vulkan-loader
    ];
  env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
}
