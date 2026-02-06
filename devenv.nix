{
  pkgs,
  lib,
  config,
  inputs,
  ...
}:

{
  packages = with pkgs; [
    glfw
    cmake
    clang
    wayland
    libxkbcommon
    xorg.libX11
    pkg-config
    libxrandr
    xorg.libX11
    xorg.libXrandr
    xorg.libXinerama
    xorg.libXcursor
    xorg.libXi
    vulkan-loader
  ];
  env.LD_LIBRARY_PATH =
    with pkgs;
    lib.makeLibraryPath [
      libGL
      xorg.libX11
      xorg.libXrandr
      xorg.libXinerama
      xorg.libXcursor
      xorg.libXi
      libxkbcommon
      libxrandr
      wayland
      vulkan-loader
    ];
  env.LIBCLANG_PATH = "${pkgs.libclang.lib}/lib";
}
