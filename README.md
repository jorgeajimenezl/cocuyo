# Cocuyo

[![CI](https://github.com/jorgeajimenezl/cocuyo/actions/workflows/build_and_test.yml/badge.svg)](https://github.com/jorgeajimenezl/cocuyo/actions/workflows/build_and_test.yml)

**Ambient lighting for your screen using WiZ smart bulbs.**

Cocuyo captures your screen in real-time and drives WiZ smart bulbs to match the colors on display — turning your room into an immersive ambient light setup.

> *Cocuyo* is the Cuban Spanish word for the click beetle (*Pyrophorus*), a bioluminescent insect native to the Caribbean.

## Features

- **Cross-platform** — Linux (PipeWire/Wayland), Windows (Graphics Capture API), and macOS (ScreenCaptureKit)
- **Zero-copy GPU pipeline** — DMA-BUF on Linux, D3D11 shared textures on Windows, IOSurface/Metal on macOS
- **Per-bulb screen regions** — assign and resize capture zones for each bulb
- **Multiple sampling strategies** — Average, Max, Min, and Palette (histogram-based dominant color)
- **GPU-accelerated sampling** — compute shaders for color extraction
- **WiZ bulb discovery** — automatic network scan with state save/restore
- **Live preview** — see the capture and region overlay before going ambient
- **System tray** — minimize to tray with quick controls (Windows/macOS)
- **Performance HUD** — real-time FPS, sampling, and dispatch metrics overlay

## Quick Start

```bash
# Build
cargo build --release

# Run
cargo run --release
```

1. Open **Bulbs** and select the WiZ bulbs on your network
2. Adjust capture regions on the preview
3. Hit **Start Ambient**

## Requirements

### Windows
- Windows 10+ with DirectX 11/12

### macOS
- macOS 13+ with a Metal-capable GPU

### Linux
- Wayland session with XDG Desktop Portal
- PipeWire
- GStreamer + dev packages (`gstreamer`, `gstreamer-app`, `gstreamer-video`, `gstreamer-allocators`)
- Vulkan runtime

## Architecture

```
Screen Capture ──► Frame (DMA-BUF / IOSurface / D3D Shared / CPU)
                        │
                        ├──► iced shader widget (preview)
                        │
                        └──► GPU Sampler (compute shaders)
                                  │
                                  └──► WiZ bulbs (UDP)
```

Built with [iced](https://github.com/iced-rs/iced) for the UI and [wgpu](https://github.com/gfx-rs/wgpu) for GPU compute and rendering.

## License

MIT
