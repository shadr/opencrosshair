# OpenCrosshair

A lightweight crosshair overlay application for Wayland compositors, inspired by HudSight.

## Requirements

- Wayland compositor with layer-shell support [check here](https://wayland.app/protocols/wlr-layer-shell-unstable-v1#compositor-support)
- Vulkan-capable GPU
- Rust toolchain (for building from source)

## Installation

### Building from Source

```bash
git clone https://github.com/yourusername/opencrosshair.git
cd opencrosshair
cargo build --release
```

The binary will be located at `target/release/opencrosshair`.

## Usage

```bash
./target/release/opencrosshair
```

## Roadmap

- [x] Add CLI configuration using [`clap`](https://docs.rs/clap)
  - [ ] Custom image path support
  - [x] Scale adjustment
  - [x] Color customization
- [ ] Explore software rendering alternatives to reduce resource usage
- [ ] Optimize RAM consumption (currently 120 MB)
- [ ] Add support for multiple monitor setups

## Contributing

Contributions are welcome! Areas of particular interest:

1. **Software rendering**: Exploring alternatives to wgpu for reduced resource usage
2. **Memory optimization**: Reducing RAM footprint
3. **Feature additions**: New configuration options and quality-of-life improvements

## License

This project is MIT licensed. See the LICENSE file for details.

## Acknowledgments

- Inspired by [HudSight](https://github.com/RESASOLUTIONS/HudSight)
- Built with the excellent [wgpu](https://wgpu.rs/) and [smithay](https://github.com/Smithay/client-toolkit) projects
